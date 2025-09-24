use crate::state::order::OrderSide;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::msg;
use solana_program::program_error::ProgramError;
use solana_program::pubkey::Pubkey;

/// Market state account - single market for test-net
/// PDA: ["mkt_v1.2", market_id(8), bump]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct MarketStateV1 {
    pub version: u8,               // =2
    pub _pad: [u8; 128],           // forward compat
    pub oracle: Pubkey,            // price oracle
    pub base_token: Pubkey,        // base token mint for collateral and PnL
    pub tick_size: u64,            // price tick size
    pub last_bid: u64,             // last bid price
    pub last_ask: u64,             // last ask price
    pub last_mark_price: u64,      // last mark price
    pub last_traded_price: u64,    // last traded price from fills
    pub open_interest: i64,        // open interest
    pub clearing_net_pos: i64,     // clearing net position
    pub clearing_entry_price: u64, // clearing entry price
    pub clearing_real_pnl: i64,    // clearing realized pnl
    pub im_bps: u16,               // initial margin basis points
    pub mm_bps: u16,               // maintenance margin basis points
    pub min_order_size: u64,       // minimum order size
    pub max_order_size: u64,       // maximum order size
    pub max_oi_size: u64,          // maximum open interest size
    pub max_user_oi_size: u64,     // maximum open interest size for a single user
    pub fill_offset: u16,          // fill offset in fixed unit terms
    pub current_log_page: u32,     // current log page to write to
    pub _pad2: [u8; 16],           // padding
    pub _pad3: [u8; 32],           // padding
    pub _pad4: [u8; 32],           // padding (total 112 bytes)
    pub _pad5: [u8; 256],          // padding
}

impl Default for MarketStateV1 {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            _pad: [0; 128],
            oracle: Pubkey::default(),
            tick_size: 0,
            last_bid: 0,
            last_ask: 0,
            last_mark_price: 0,
            last_traded_price: 0,
            open_interest: 0,
            clearing_net_pos: 0,
            clearing_entry_price: 0,
            clearing_real_pnl: 0,
            im_bps: 0,
            mm_bps: 0,
            min_order_size: 0,
            max_order_size: 0,
            max_oi_size: 0,
            max_user_oi_size: 0,
            base_token: Pubkey::default(),
            fill_offset: 0,
            current_log_page: 0,
            _pad2: [0; 16],
            _pad3: [0; 32],
            _pad4: [0; 32],
            _pad5: [0; 256],
        }
    }
}

impl MarketStateV1 {
    pub const CURRENT_VERSION: u8 = 3; // layout changed - added last_traded_price

    /// Get the mid price
    pub fn mid_price(&self) -> u64 {
        if self.last_bid == 0 || self.last_ask == 0 {
            0
        } else {
            (self.last_bid + self.last_ask) / 2
        }
    }

    /// Get the spread
    pub fn spread(&self) -> u64 {
        if self.last_bid == 0 || self.last_ask == 0 {
            0
        } else {
            self.last_ask - self.last_bid
        }
    }

    /// Check if market is active (has recent prices)
    pub fn is_active(&self) -> bool {
        self.last_bid > 0 && self.last_ask > 0
    }

    /// Apply a fill to the market state – updates open interest, clearing position, and last traded price.
    pub fn process_fill(
        &mut self,
        fill: &crate::state::order::OrderFillResult,
        cma_fill_result: &crate::state::cma::CmaFillResult,
    ) -> Result<(), solana_program::program_error::ProgramError> {
        // Update last traded price to the fill's weighted average price
        self.last_traded_price = fill.weighted_avg_price;

        // Update open interest (saturating to avoid overflow)
        let open_interest_change =
            cma_fill_result.new_net_position.abs() - cma_fill_result.old_net_position.abs();
        self.open_interest = self.open_interest.saturating_add(open_interest_change);

        if (self.open_interest > 0) && ((self.open_interest as u64) > self.max_oi_size) {
            if open_interest_change < 0 {
                msg!(
                    "Open interest exceeds max size, but reduction allowed {} > {} (delta={})",
                    self.open_interest,
                    self.max_oi_size,
                    open_interest_change
                );
            } else {
                msg!(
                    "Error: Open interest exceeds max size {} > {}",
                    self.open_interest,
                    self.max_oi_size
                );
                return Err(ProgramError::InvalidArgument);
            }
        }

        let fill_qty = fill.filled_qty;
        let fill_price = fill.weighted_avg_price;

        // Update clearing position based on order side
        // When user buys (Bid), clearing goes short (negative)
        // When user sells (Ask), clearing goes long (positive)
        let clearing_position_change = match fill.side {
            OrderSide::Bid => -(fill_qty as i64), // User buys, clearing sells
            OrderSide::Ask => fill_qty as i64,    // User sells, clearing buys
        };

        // Update clearing position and entry price
        let old_clearing_pos = self.clearing_net_pos;
        let new_clearing_pos = old_clearing_pos + clearing_position_change;

        // Calculate new weighted average entry price if position increases
        if (old_clearing_pos >= 0 && clearing_position_change > 0)
            || (old_clearing_pos <= 0 && clearing_position_change < 0)
        {
            // Position is increasing in same direction, calculate weighted average
            let old_notional = old_clearing_pos.unsigned_abs() * self.clearing_entry_price;
            let new_notional = fill_qty * fill_price;
            let total_notional = old_notional + new_notional;
            let total_position = old_clearing_pos.unsigned_abs() + fill_qty;

            if total_position > 0 {
                self.clearing_entry_price = total_notional / total_position;
            }
        } else if old_clearing_pos != 0
            && ((old_clearing_pos > 0 && clearing_position_change < 0)
                || (old_clearing_pos < 0 && clearing_position_change > 0))
        {
            // Position is reducing or flipping, realize PnL
            let reduction_qty = fill_qty.min(old_clearing_pos.unsigned_abs());
            let realized_pnl = if old_clearing_pos > 0 {
                // Clearing was long, now selling
                (fill_price as i128 - self.clearing_entry_price as i128) * reduction_qty as i128
                    / 100_000_000i128
            } else {
                // Clearing was short, now buying
                (self.clearing_entry_price as i128 - fill_price as i128) * reduction_qty as i128
                    / 100_000_000i128
            } as i64;

            self.clearing_real_pnl = self.clearing_real_pnl.saturating_add(realized_pnl);

            // If position flips, set new entry price
            if new_clearing_pos != 0 && (old_clearing_pos > 0) != (new_clearing_pos > 0) {
                self.clearing_entry_price = fill_price;
            }
        } else if old_clearing_pos == 0 {
            // Starting from zero position
            self.clearing_entry_price = fill_price;
        }

        self.clearing_net_pos = new_clearing_pos;

        // Reset entry price when position becomes zero
        if new_clearing_pos == 0 {
            self.clearing_entry_price = 0;
        }

        msg!(
            "Updated clearing position: old={}, change={}, new={}, entry_price={}, realized_pnl={}",
            old_clearing_pos,
            clearing_position_change,
            new_clearing_pos,
            self.clearing_entry_price,
            self.clearing_real_pnl
        );

        Ok(())
    }

    /// Validates order against market requirements (minimum size, etc.)
    pub fn validate_order_conformance(
        &self,
        qty: u64,
        price: u64,
    ) -> Result<(), solana_program::program_error::ProgramError> {
        use solana_program::{msg, program_error::ProgramError};

        // Check minimum order size
        if qty < self.min_order_size {
            msg!(
                "Error: Order quantity {} below minimum size {}",
                qty,
                self.min_order_size
            );
            return Err(ProgramError::InvalidArgument);
        }

        // Check maximum order size
        if self.max_order_size > 0 && qty > self.max_order_size {
            msg!(
                "Error: Order quantity {} exceeds maximum size {}",
                qty,
                self.max_order_size
            );
            return Err(ProgramError::InvalidArgument);
        }

        let is_mkt_order: bool = price == 0;

        // Check price aligns with tick size (only for limit orders)
        if !is_mkt_order && price % self.tick_size != 0 {
            msg!(
                "Error: Price {} not aligned with tick size {}",
                price,
                self.tick_size
            );
            return Err(ProgramError::InvalidArgument);
        }

        if self.last_mark_price == 0 {
            msg!("Error: Cannot enter order when market has a mark price of 0");
            return Err(ProgramError::InvalidArgument);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::{program_error::ProgramError, pubkey::Pubkey};

    fn create_test_market_state() -> MarketStateV1 {
        MarketStateV1 {
            oracle: Pubkey::new_unique(),
            tick_size: 1000,           // 0.001 price increment
            last_bid: 100_000,         // $100
            last_ask: 101_000,         // $101
            last_mark_price: 100_500,  // $100.50
            im_bps: 1000,              // 10% initial margin
            mm_bps: 500,               // 5% maintenance margin
            min_order_size: 1_000_000, // 1.0 tokens minimum
            base_token: Pubkey::new_unique(),
            ..Default::default()
        }
    }

    #[test]
    fn test_validate_order_conformance_valid_order() {
        let market = create_test_market_state();

        // Valid order: above min size, valid price, aligned with tick
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens (above 1.0 min)
            100_000,   // $100 (multiple of 1000 tick size)
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_order_conformance_below_min_size() {
        let market = create_test_market_state();

        // Order below minimum size
        let result = market.validate_order_conformance(
            500_000, // 0.5 tokens (below 1.0 min)
            100_000, // $100
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::InvalidArgument);
    }

    #[test]
    fn test_validate_order_conformance_zero_price() {
        let market = create_test_market_state();

        // Zero price should be valid for market orders
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            0,         // Market order with 0 price
        );

        assert!(result.is_ok(), "Market orders with 0 price should be valid");
    }

    #[test]
    fn test_validate_order_conformance_price_not_aligned() {
        let market = create_test_market_state();

        // Price not aligned with tick size
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            100_500,   // $100.50 (not multiple of 1000)
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::InvalidArgument);
    }

    #[test]
    fn test_validate_order_conformance_max_order_size() {
        let mut market = create_test_market_state();
        market.max_order_size = 5_000_000; // 5.0 tokens max

        // Valid order: at max size
        let result = market.validate_order_conformance(
            5_000_000, // Exactly 5.0 tokens (at max)
            100_000,   // $100
        );
        assert!(result.is_ok());

        // Invalid order: above max size
        let result = market.validate_order_conformance(
            5_000_001, // 5.000001 tokens (above max)
            100_000,   // $100
        );
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::InvalidArgument);

        // When max_order_size is 0 (unlimited), large orders should pass
        market.max_order_size = 0;
        let result = market.validate_order_conformance(
            1_000_000_000_000, // Very large order
            100_000,           // $100
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_order_conformance_edge_cases() {
        let market = create_test_market_state();

        // Test exactly at minimum size
        let result = market.validate_order_conformance(
            1_000_000, // Exactly 1.0 tokens (at min)
            101_000,   // $101 (aligned)
        );
        assert!(result.is_ok());

        // Test large valid order
        let result = market.validate_order_conformance(
            1_000_000_000, // 1000 tokens
            999_000,       // $999 (aligned with tick)
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_market_state_mid_price() {
        let market = create_test_market_state();
        assert_eq!(market.mid_price(), 100_500); // (100_000 + 101_000) / 2

        // Test with zero bid
        let mut market_zero_bid = market.clone();
        market_zero_bid.last_bid = 0;
        assert_eq!(market_zero_bid.mid_price(), 0);

        // Test with zero ask
        let mut market_zero_ask = market.clone();
        market_zero_ask.last_ask = 0;
        assert_eq!(market_zero_ask.mid_price(), 0);
    }

    #[test]
    fn test_market_state_spread() {
        let market = create_test_market_state();
        assert_eq!(market.spread(), 1_000); // 101_000 - 100_000

        // Test with zero bid
        let mut market_zero_bid = market.clone();
        market_zero_bid.last_bid = 0;
        assert_eq!(market_zero_bid.spread(), 0);

        // Test with zero ask
        let mut market_zero_ask = market.clone();
        market_zero_ask.last_ask = 0;
        assert_eq!(market_zero_ask.spread(), 0);
    }

    #[test]
    fn test_market_state_is_active() {
        let market = create_test_market_state();
        assert!(market.is_active()); // Both bid and ask > 0

        // Test with zero bid
        let mut market_zero_bid = market.clone();
        market_zero_bid.last_bid = 0;
        assert!(!market_zero_bid.is_active());

        // Test with zero ask
        let mut market_zero_ask = market.clone();
        market_zero_ask.last_ask = 0;
        assert!(!market_zero_ask.is_active());

        // Test with both zero
        let mut market_inactive = market.clone();
        market_inactive.last_bid = 0;
        market_inactive.last_ask = 0;
        assert!(!market_inactive.is_active());
    }

    #[test]
    fn test_validate_order_conformance_various_tick_sizes() {
        // Test with different tick size
        let mut market = create_test_market_state();
        market.tick_size = 100; // 0.0001 increment

        // Should work with price aligned to new tick size
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            100_100,   // $100.01 (multiple of 100)
        );
        assert!(result.is_ok());

        // Should fail with price not aligned to new tick size
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            100_150,   // $100.015 (not multiple of 100)
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_order_conformance_market_orders() {
        let market = create_test_market_state();

        // Market order with 0 price should be valid
        let result = market.validate_order_conformance(
            5_000_000, // 5.0 tokens
            0,         // Market order (price = 0)
        );
        assert!(result.is_ok(), "Market orders with price=0 should be valid");

        // Market order still respects minimum size
        let result = market.validate_order_conformance(
            500_000, // 0.5 tokens (below minimum)
            0,       // Market order
        );
        assert!(
            result.is_err(),
            "Market orders must still respect minimum size"
        );
        assert_eq!(result.unwrap_err(), ProgramError::InvalidArgument);
    }

    #[test]
    fn test_validate_order_conformance_market_order_tick_size() {
        let mut market = create_test_market_state();
        market.tick_size = 100; // 0.0001 increment

        // Market orders should not be subject to tick size validation
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            0,         // Market order
        );
        assert!(
            result.is_ok(),
            "Market orders should bypass tick size validation"
        );

        // Large market order should also be valid
        let result = market.validate_order_conformance(
            100_000_000, // 100 tokens
            0,           // Market order
        );
        assert!(result.is_ok(), "Large market orders should be valid");
    }

    #[test]
    fn test_market_state_serialization() {
        use borsh::{BorshDeserialize, BorshSerialize};

        let market = MarketStateV1::default();
        let serialized = market.try_to_vec().unwrap();
        println!("Serialized size: {}", serialized.len());
        println!("Struct size: {}", std::mem::size_of::<MarketStateV1>());

        // Test round-trip
        let deserialized = MarketStateV1::try_from_slice(&serialized).unwrap();
        assert_eq!(market.version, deserialized.version);
    }

    #[test]
    fn test_validate_order_conformance_limit_vs_market() {
        let market = create_test_market_state();

        // Valid limit order
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            100_000,   // $100 (aligned with tick size)
        );
        assert!(
            result.is_ok(),
            "Limit order with valid price should succeed"
        );

        // Valid market order
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            0,         // Market order
        );
        assert!(result.is_ok(), "Market order with price=0 should succeed");

        // Invalid limit order (not aligned with tick)
        let result = market.validate_order_conformance(
            2_000_000, // 2.0 tokens
            100_123,   // Not aligned with 1000 tick size
        );
        assert!(
            result.is_err(),
            "Limit order with misaligned price should fail"
        );
    }
}
