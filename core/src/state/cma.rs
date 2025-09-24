use crate::state::math::mul_qty_px_signed;
use crate::state::math::mul_qty_px_to_notional;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;
use solana_program::{msg, program_error::ProgramError};

#[cfg(test)]
#[path = "cma_test.rs"]
mod cma_test;

/// Free (non-committed) collateral per SPL-Token mint.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct TokenBalance {
    pub mint: Pubkey,
    pub amount: u64, // free collateral for this mint
    pub _pad: [u8; 16],
}

/// Indicates where a margin/position bucket applies.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub enum MarginScope {
    /// Collateral committed to a single isolated market (identified by its numeric id).
    MarketIsolated(u64),
    // Future scopes (FullCross, CrossGroup, etc.) can be added here while preserving Borsh order.
}

/// Holds committed collateral, reserved margin, and position size for a given scope & token.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct MarginBucket {
    pub scope: MarginScope,
    pub mint: Pubkey,

    /// Committed collateral: amount of collateral that is committed to the margin account
    /// WARNING: This value should always be in 10^6 decimilization *REGARDLESS* of the token's decimals
    ///
    pub committed: u64,
    /// Net position: positive for long, negative for short
    pub net_position: i64,
    /// Total absolute quantity of open bid orders
    pub open_bid_qty: u64,
    /// Total absolute quantity of open ask orders
    pub open_ask_qty: u64,
    /// Weighted average entry price of the current net position
    pub avg_entry_price: u64,
    /// The user selected initial margin (note does not override market margin)
    /// if the latter is tighter
    pub user_set_im_bps: u16,
    pub _pad: [u8; 32],
}

/// Cross margin account – now supports multiple tokens and multiple margin scopes.
/// PDA: ["cma_v1.2", user_pubkey, bump]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct CrossMarginAccountV1 {
    pub version: u8,     // =2 – layout bumped
    pub _pad: [u8; 128], // forward compat / alignment

    pub user: Pubkey,                // user pubkey for indexing convenience
    pub balances: Vec<TokenBalance>, // all free collateral by mint
    pub buckets: Vec<MarginBucket>,  // committed/reserved/position grouped by scope

    pub _pad2: [u8; 8], // forward compat / alignment
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq, Default)]
pub struct CmaFillResult {
    pub new_net_position: i64,
    pub old_net_position: i64,
    pub realized_pnl_banked: i64,
}

impl CrossMarginAccountV1 {
    pub const CURRENT_VERSION: u8 = 2;

    /// Convenience – locate (or lazily create) a free-collateral bucket for given mint.
    pub fn balance_for_mut(&mut self, mint: &Pubkey) -> &mut TokenBalance {
        if let Some(pos) = self.balances.iter().position(|tb| &tb.mint == mint) {
            &mut self.balances[pos]
        } else {
            self.balances.push(TokenBalance {
                mint: *mint,
                amount: 0,
                _pad: [0; 16],
            });
            self.balances.last_mut().unwrap()
        }
    }

    /// Update open order quantities when cancelling an order
    pub fn update_collateral_on_cancel(
        &mut self,
        market_id: u64,
        side: &crate::state::order::OrderSide,
        unfilled_qty: u64,
    ) -> Result<(), &'static str> {
        use crate::state::order::OrderSide;

        // Find the market-specific bucket
        let market_bucket = self
            .buckets
            .iter_mut()
            .find(
                |bucket| matches!(bucket.scope, MarginScope::MarketIsolated(id) if id == market_id),
            )
            .ok_or("Market bucket not found")?;

        // Update open order quantities based on order side
        // Cancels don't affect net_position, only open order quantities
        match side {
            OrderSide::Bid => {
                market_bucket.open_bid_qty =
                    market_bucket.open_bid_qty.saturating_sub(unfilled_qty);
            }
            OrderSide::Ask => {
                market_bucket.open_ask_qty =
                    market_bucket.open_ask_qty.saturating_sub(unfilled_qty);
            }
        }

        Ok(())
    }

    pub fn bucket_for_view(&self, scope: &MarginScope, mint: &Pubkey) -> Option<&MarginBucket> {
        if let Some(pos) = self
            .buckets
            .iter()
            .position(|b| &b.scope == scope && &b.mint == mint)
        {
            Some(&self.buckets[pos])
        } else {
            None
        }
    }

    /// Locate (or create) margin bucket for a given scope & mint.
    pub fn bucket_for_mut(&mut self, scope: &MarginScope, mint: &Pubkey) -> &mut MarginBucket {
        if let Some(pos) = self
            .buckets
            .iter()
            .position(|b| &b.scope == scope && &b.mint == mint)
        {
            &mut self.buckets[pos]
        } else {
            self.buckets.push(MarginBucket {
                scope: scope.clone(),
                mint: *mint,
                committed: 0,
                net_position: 0,
                open_bid_qty: 0,
                open_ask_qty: 0,
                avg_entry_price: 0,
                user_set_im_bps: 0,
                _pad: [0; 32],
            });
            self.buckets.last_mut().unwrap()
        }
    }

    /// Aggregate committed + free across all mints.
    pub fn total_collateral(&self, mint: &Pubkey) -> u64 {
        let free_sum: u64 = self
            .balances
            .iter()
            .filter(|tb| &tb.mint == mint)
            .map(|tb| tb.amount)
            .sum();
        let committed_sum: u64 = self
            .buckets
            .iter()
            .filter(|b| &b.mint == mint)
            .map(|b| b.committed)
            .sum();
        free_sum.saturating_add(committed_sum)
    }

    /// Available for new commitments: sum of free balances minus account-level reserved.
    pub fn uncommitted_collateral(&self, mint: &Pubkey) -> u64 {
        self.balances
            .iter()
            .filter(|tb| &tb.mint == mint)
            .map(|tb| tb.amount)
            .sum()
    }

    /// Validates and updates collateral requirements for an order
    /// NOTE: One thing to be aware of is that price should always be based on the *market price* not the user
    ///      supplied price in their order. Otherwise users could access very low collateral requirements since
    ///      they can set price in their orders.
    pub fn validate_and_update_collateral(
        &mut self,
        market_state: &crate::MarketStateV1,
        market_id: u64,
        side: crate::state::order::OrderSide,
        qty: u64,
        is_liquidation: bool,
    ) -> Result<(), solana_program::program_error::ProgramError> {
        // Find or create margin bucket for this market
        let market_scope = MarginScope::MarketIsolated(market_id);
        let bucket = self.bucket_for_mut(&market_scope, &market_state.base_token);

        if is_liquidation {
            bucket.update_open_order_qty(side, qty)?;
        } else {
            bucket.validate_and_update_open_order_qty(market_state, side, qty)?;
        }

        msg!("Collateral validated: net_position={}, open_bid_qty={}, open_ask_qty={} qty={} user_set_im_bps={}", 
            bucket.net_position, bucket.open_bid_qty, bucket.open_ask_qty, qty, bucket.user_set_im_bps);

        Ok(())
    }

    pub fn qty_left_for_margin(
        &self,
        market_state: &crate::MarketStateV1,
        market_id: u64,
        side: crate::state::order::OrderSide,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        let bucket = self.bucket_for_view(
            &MarginScope::MarketIsolated(market_id),
            &market_state.base_token,
        );
        match bucket {
            Some(bucket) => bucket.qty_left_for_margin(market_state, side),
            None => Ok(0),
        }
    }

    /// Process a fill and update position tracking
    pub fn process_fill(
        &mut self,
        market_id: u64,
        side: crate::state::order::OrderSide,
        qty: u64,
        price: u64,
        mint: &Pubkey,
    ) -> Result<CmaFillResult, solana_program::program_error::ProgramError> {
        use crate::state::position::{process_fill, Fill};
        use solana_program::{msg, program_error::ProgramError};

        // Find the market-specific bucket
        let bucket = self
            .buckets
            .iter_mut()
            .find(|b| {
                matches!(&b.scope, MarginScope::MarketIsolated(id) if *id == market_id)
                    && &b.mint == mint
            })
            .ok_or_else(|| {
                msg!("Error: Market bucket not found for fill");
                ProgramError::InvalidAccountData
            })?;

        // Create fill struct
        let fill = Fill { side, qty, price };

        // Process the fill
        let fill_result = process_fill(bucket.net_position, bucket.avg_entry_price, &fill)?;

        let old_net_position = bucket.net_position;

        // Update bucket with results
        bucket.net_position = fill_result.new_net_position;
        bucket.avg_entry_price = fill_result.new_avg_entry_price;

        // Update open order quantities
        match side {
            crate::state::order::OrderSide::Bid => {
                bucket.open_bid_qty = bucket.open_bid_qty.saturating_sub(qty);
            }
            crate::state::order::OrderSide::Ask => {
                bucket.open_ask_qty = bucket.open_ask_qty.saturating_sub(qty);
            }
        }

        // Handle realized PnL if any
        if fill_result.realized_pnl != 0 {
            // Apply realized PnL to committed capital
            if fill_result.realized_pnl > 0 {
                // Profit increases committed capital
                bucket.committed = bucket
                    .committed
                    .checked_add(fill_result.realized_pnl as u64)
                    .ok_or_else(|| {
                        msg!("Error: Overflow adding realized profit");
                        ProgramError::ArithmeticOverflow
                    })?;
            } else {
                // Loss reduces committed capital
                let loss = (-fill_result.realized_pnl) as u64;
                if bucket.committed < loss {
                    msg!(
                        "Error: Warning: Loss {} exceeds committed capital. Current: {}",
                        loss,
                        bucket.committed
                    );
                    bucket.committed = 0;
                } else {
                    bucket.committed = bucket.committed.saturating_sub(loss);
                }
            }

            msg!(
                "Fill processed: realized PnL = {}, new committed = {}",
                fill_result.realized_pnl,
                bucket.committed
            );
        }

        msg!("Fill processed: new net_position={}, avg_entry_price={}, open_bid_qty={}, open_ask_qty={}", 
             bucket.net_position, bucket.avg_entry_price, bucket.open_bid_qty, bucket.open_ask_qty);

        Ok(CmaFillResult {
            new_net_position: bucket.net_position,
            old_net_position,
            realized_pnl_banked: fill_result.realized_pnl,
        })
    }

    /// Calculate equity for a margin bucket given a mark price
    pub fn calculate_bucket_equity(
        &self,
        market_id: u64,
        mint: &Pubkey,
        mark_price: u64,
    ) -> Result<i64, solana_program::program_error::ProgramError> {
        use crate::state::position::calculate_equity;

        // Find the market-specific bucket
        let bucket = self
            .buckets
            .iter()
            .find(|b| {
                matches!(&b.scope, MarginScope::MarketIsolated(id) if *id == market_id)
                    && &b.mint == mint
            })
            .ok_or(solana_program::program_error::ProgramError::InvalidAccountData)?;

        calculate_equity(
            bucket.committed,
            bucket.net_position,
            bucket.avg_entry_price,
            mark_price,
        )
    }

    pub fn net_position(
        &self,
        market_id: u64,
        mint: &Pubkey,
    ) -> Result<i64, solana_program::program_error::ProgramError> {
        let bucket = self.bucket_for_view(&MarginScope::MarketIsolated(market_id), mint);
        match bucket {
            Some(bucket) => Ok(bucket.net_position),
            None => Ok(0),
        }
    }
}

impl MarginBucket {
    pub fn is_empty(&self) -> bool {
        self.committed == 0
            && self.net_position == 0
            && self.open_bid_qty == 0
            && self.open_ask_qty == 0
    }

    pub fn is_open(&self) -> bool {
        self.net_position != 0 || self.open_bid_qty != 0 || self.open_ask_qty != 0
    }

    pub fn validate_and_update_open_order_qty(
        &mut self,
        market_state: &crate::MarketStateV1,
        side: crate::state::order::OrderSide,
        qty: u64,
    ) -> Result<(), solana_program::program_error::ProgramError> {
        let worst_case_usage = self.worst_case_direction_add(side, qty)?;

        // Check if worst case position would exceed max_user_oi_size
        if market_state.max_user_oi_size > 0 && worst_case_usage > market_state.max_user_oi_size {
            msg!(
                "Error: Order would exceed maximum user open interest. Worst case position: {}, Max allowed: {}",
                worst_case_usage,
                market_state.max_user_oi_size,
            );
            return Err(ProgramError::InvalidArgument);
        }

        let required_margin = self.calc_required_margin_mkt(market_state, worst_case_usage)?;
        let equity = self.calc_equity(market_state.last_mark_price)?;

        // Check if sufficient committed collateral
        if equity < required_margin {
            msg!(
                "Error: Insufficient collateral. Required: {}, Available: {}, Worst case usage: {}",
                required_margin,
                self.committed,
                worst_case_usage,
            );
            return Err(ProgramError::InsufficientFunds);
        }

        return self.update_open_order_qty(side, qty);
    }

    pub fn update_open_order_qty(
        &mut self,
        side: crate::state::order::OrderSide,
        qty: u64,
    ) -> Result<(), solana_program::program_error::ProgramError> {
        match side {
            crate::state::order::OrderSide::Bid => {
                self.open_bid_qty = self.open_bid_qty.checked_add(qty).ok_or_else(|| {
                    msg!("Error: Overflow updating open bid quantity");
                    ProgramError::ArithmeticOverflow
                })?;
            }
            crate::state::order::OrderSide::Ask => {
                self.open_ask_qty = self.open_ask_qty.checked_add(qty).ok_or_else(|| {
                    msg!("Error: Overflow updating open ask quantity");
                    ProgramError::ArithmeticOverflow
                })?;
            }
        }
        Ok(())
    }

    pub fn qty_left_for_margin(
        &self,
        market_state: &crate::MarketStateV1,
        side: crate::state::order::OrderSide,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        let worst_case_usage = self.worst_case_direction(side)?;
        let effective_im_bps = market_state.im_bps.max(self.user_set_im_bps);

        let equity = self.calc_equity(market_state.last_mark_price)?;

        let capacity128 = equity as u128 * 10_000 as u128 * 100_000_000 as u128
            / (effective_im_bps as u128 * market_state.last_mark_price as u128);

        let capacity = i64::try_from(capacity128).map_err(|_| ProgramError::ArithmeticOverflow)?;
        let qty_left = (capacity - worst_case_usage).max(0) as u64;

        return Ok(qty_left);
    }

    pub fn calc_required_margin_mkt(
        &self,
        market_state: &crate::MarketStateV1,
        usage: u64,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        self.calc_required_margin(market_state.last_mark_price, market_state.im_bps, usage)
    }

    pub fn calc_required_margin(
        &self,
        last_mark_price: u64,
        mkt_im_bps: u16,
        usage: u64,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        let effective_im_bps = mkt_im_bps.max(self.user_set_im_bps);
        let notional_value = mul_qty_px_to_notional(usage, last_mark_price)?;

        notional_value
            .checked_mul(effective_im_bps as u64)
            .and_then(|x| x.checked_div(10000))
            .ok_or_else(|| {
                msg!("Error: Overflow calculating required margin");
                ProgramError::ArithmeticOverflow
            })
    }

    pub fn worst_case_direction_add(
        &self,
        side: crate::state::order::OrderSide,
        add_qty: u64,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        let usage = self.worst_case_direction(side)?;
        let add_qty_i64 = i64::try_from(add_qty).map_err(|_| ProgramError::ArithmeticOverflow)?;
        let sum = usage
            .checked_add(add_qty_i64)
            .ok_or(ProgramError::ArithmeticOverflow)?;
        Ok(sum.max(0) as u64)
    }

    pub fn worst_case_direction(
        &self,
        side: crate::state::order::OrderSide,
    ) -> Result<i64, solana_program::program_error::ProgramError> {
        Ok(match side {
            crate::state::order::OrderSide::Bid => self.net_position + (self.open_bid_qty) as i64,
            crate::state::order::OrderSide::Ask => {
                (-self.net_position) + (self.open_ask_qty) as i64
            }
        })
    }

    /// Calculate the worst case position magnitude considering open orders
    /// Returns max(net_position + open_bid_qty, -net_position + open_ask_qty)
    pub fn worst_case_position(&self) -> u64 {
        // Worst case long: all bids fill
        let worst_long = self.net_position.saturating_add(self.open_bid_qty as i64);

        // Worst case short: all asks fill (position becomes more negative)
        let worst_short = (-self.net_position).saturating_add(self.open_ask_qty as i64);

        // Return the larger absolute value
        worst_long.abs().max(worst_short.abs()) as u64
    }

    /// Calculate the maximum amount that can be uncommitted from this bucket
    /// given the current market conditions and margin requirements
    pub fn calculate_uncommittable_amount(
        &self,
        last_mark_price: u64,
        im_bps: u16,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        use solana_program::{msg, program_error::ProgramError};

        let worst_case_pos = self.worst_case_position();

        // Calculate required collateral: worst_case_position * last_mark_price * im_bps / 10000
        let notional = mul_qty_px_to_notional(worst_case_pos, last_mark_price)?;

        let required_collateral = notional
            .checked_mul(im_bps as u64)
            .and_then(|x| x.checked_div(10000))
            .ok_or_else(|| {
                msg!("Error: Overflow calculating required collateral");
                ProgramError::ArithmeticOverflow
            })?;

        let equity = self.calc_equity(last_mark_price)?;

        // Calculate uncommittable amount = equity - required_collateral
        // Can be negative (returns 0 via saturating_sub)
        let uncommittable = equity.saturating_sub(required_collateral);

        msg!(
            "Uncommittable calculation: worst_case={}, required_collateral={}, equity={}, uncommittable={}",
            worst_case_pos, required_collateral, equity, uncommittable
        );

        Ok(uncommittable)
    }

    pub fn calc_equity(
        &self,
        last_mark_price: u64,
    ) -> Result<u64, solana_program::program_error::ProgramError> {
        // Calculate unrealized PnL (scaled to collateral decimals)
        let unrealized_pnl = if self.net_position != 0 {
            let price_diff = if self.net_position > 0 {
                last_mark_price as i64 - self.avg_entry_price as i64
            } else {
                self.avg_entry_price as i64 - last_mark_price as i64
            };
            // Use helper to scale qty*price_diff / 1e8
            mul_qty_px_signed(self.net_position.abs(), price_diff)?
        } else {
            0
        };

        // Calculate equity = committed + unrealized_pnl
        let equity = if unrealized_pnl >= 0 {
            self.committed
                .checked_add(unrealized_pnl as u64)
                .ok_or_else(|| {
                    msg!("Error: Overflow calculating equity");
                    ProgramError::ArithmeticOverflow
                })?
        } else {
            let loss = (-unrealized_pnl) as u64;
            self.committed.saturating_sub(loss)
        };
        Ok(equity)
    }

    /// Default constructor for MarginBucket
    pub fn new(scope: MarginScope, mint: Pubkey) -> Self {
        Self {
            scope,
            mint,
            committed: 0,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        }
    }
}
