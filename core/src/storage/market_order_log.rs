use crate::state::cma::CmaFillResult;
use crate::state::order::OrderDetails;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{msg, program_error::ProgramError, pubkey::Pubkey};

/// Type of update that generated this log entry
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq)]
pub enum OrderUpdateType {
    /// Initial order placement
    OrderEntry,
    /// Order cancellation
    Cancel,
    /// Partial or full fill
    Fill,
    /// Order liquidation
    Liquidation,
    /// Close position entry
    ClosePosition,

    /// User Collateral Update
    /// Note that this update does *NOT* represent an order related event and consumers of the
    /// log should *NOT* process it as an order, use the order_id, or try to map to an order.
    ///
    /// This is a bit of a hacked entry to allow for user related collateral commit/uncommits
    /// These updates are not tied to a specific order, so instead they hack the order entry
    /// fields to store the collateral snapshot and changes. The fields used are:
    /// - user: the user who owns the collateral
    /// - fill_details.qty: the collateral snapshot
    UserCollateralUpdate,

    /// Reserved for future use
    #[allow(dead_code)]
    Reserved(u8),
}

/// A single entry in the market-wide order log
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug)]
pub struct OrderLogEntry {
    /// The user who owns this order
    pub user: Pubkey,
    /// The unique order ID (unique per user per market)
    pub order_id: u64,
    /// The type of update that created this entry
    pub update_type: OrderUpdateType,
    /// The order details at the time of this update
    pub order_details: OrderDetails,
    /// The fill details at the time of this update (if fill event)
    pub fill_details: FillLogDetails,
    /// Solana slot of when this entry was created
    pub slot: u64,
}

/// A single entry in the market-wide order log
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default)]
pub struct FillLogDetails {
    /// Fill price
    pub price: u64,
    /// Fill quantity
    pub qty: u64,
    /// User position after fill
    pub account: CmaFillResult,
}

/// Market-wide append-only order log
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct MarketOrderLog {
    /// The market ID this log belongs to
    pub market_id: u64,
    /// The page number (for pagination when exceeding 10MB)
    pub page: u32,
    /// Total number of entries in this page
    pub entry_count: u64,
    /// The actual log entries
    pub entries: Vec<OrderLogEntry>,
}

pub const EMPTY_FILL_DETAILS: FillLogDetails = FillLogDetails {
    price: 0,
    qty: 0,
    account: CmaFillResult {
        new_net_position: 0,
        old_net_position: 0,
        realized_pnl_banked: 0,
    },
};

impl OrderLogEntry {
    pub fn synth_user_collateral_update(user: Pubkey, collateral_snapshot: u64, slot: u64) -> Self {
        Self {
            user,
            fill_details: FillLogDetails {
                qty: collateral_snapshot,
                ..Default::default()
            },
            slot,
            order_id: 0,
            update_type: OrderUpdateType::UserCollateralUpdate,
            order_details: OrderDetails::default(),
        }
    }
}

impl MarketOrderLog {
    /// Create a new empty market order log
    pub fn new(market_id: u64, page: u32) -> Self {
        Self {
            market_id,
            page,
            entry_count: 0,
            entries: Vec::new(),
        }
    }

    pub fn append_user_collateral_update(
        &mut self,
        user: Pubkey,
        collateral_snapshot: u64,
        slot: u64,
    ) -> Result<(), ProgramError> {
        let entry = OrderLogEntry::synth_user_collateral_update(user, collateral_snapshot, slot);
        self.entries.push(entry);
        self.entry_count += 1;

        msg!(
            "Appended user collateral update: market_id={}, page={}, entry_count={}, user={}, collateral_snapshot={}",
            self.market_id,
            self.page,
            self.entry_count,
            user,
            collateral_snapshot
        );
        Ok(())
    }

    /// Append a new entry to the log
    pub fn append_entry(
        &mut self,
        user: Pubkey,
        order_id: u64,
        update_type: OrderUpdateType,
        order_details: OrderDetails,
        fill_details: Option<FillLogDetails>,
        slot: u64,
    ) -> Result<(), ProgramError> {
        let fill_logged = match fill_details {
            Some(fill_details) => fill_details,
            None => EMPTY_FILL_DETAILS,
        };

        let entry = OrderLogEntry {
            user,
            order_id,
            update_type,
            order_details,
            fill_details: fill_logged,
            slot,
        };

        self.entries.push(entry);
        self.entry_count += 1;

        msg!(
            "Appended order log entry: market_id={}, page={}, entry_count={}, user={}, order_id={}, update_type={:?}",
            self.market_id,
            self.page,
            self.entry_count,
            user,
            order_id,
            update_type
        );

        Ok(())
    }

    /// Check if the log needs reallocation
    pub fn needs_realloc(&self, current_account_size: usize) -> bool {
        let estimated_size = self.estimated_serialized_size();
        let buffer = 1000; // Safety buffer
        estimated_size + buffer > current_account_size
    }

    /// Estimate the serialized size of the log
    pub fn estimated_serialized_size(&self) -> usize {
        // Base struct size
        let base_size = 8 + 4 + 8; // market_id + page + entry_count

        // Vector overhead
        let vec_overhead = 4; // Borsh vector length prefix

        // Estimate per entry (this is an approximation)
        let per_entry_size = 32 + 8 + 1 + std::mem::size_of::<OrderDetails>() + 8;

        base_size + vec_overhead + (self.entries.len() * per_entry_size)
    }

    /// Get statistics about the log
    pub fn get_stats(&self) -> MarketOrderLogStats {
        let mut order_entries = 0;
        let mut cancels = 0;
        let mut fills = 0;
        let mut liquidations = 0;
        let mut close_positions = 0;
        let mut user_collateral_updates = 0;
        let mut other = 0;

        for entry in &self.entries {
            match entry.update_type {
                OrderUpdateType::OrderEntry => order_entries += 1,
                OrderUpdateType::Cancel => cancels += 1,
                OrderUpdateType::Fill => fills += 1,
                OrderUpdateType::Liquidation => liquidations += 1,
                OrderUpdateType::ClosePosition => close_positions += 1,
                OrderUpdateType::UserCollateralUpdate => user_collateral_updates += 1,
                OrderUpdateType::Reserved(_) => other += 1,
            }
        }

        MarketOrderLogStats {
            total_entries: self.entry_count,
            order_entries,
            cancels,
            fills,
            liquidations,
            close_positions,
            user_collateral_updates,
            other,
        }
    }
}

/// Statistics about a market order log
#[derive(Debug)]
pub struct MarketOrderLogStats {
    pub total_entries: u64,
    pub order_entries: u64,
    pub cancels: u64,
    pub fills: u64,
    pub liquidations: u64,
    pub close_positions: u64,
    pub user_collateral_updates: u64,
    pub other: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::order::{
        OrderOriginator, OrderPrice, OrderSide, OrderTombstone, TriggerCondition, TriggerEntrySize,
    };

    #[test]
    fn test_market_order_log_new() {
        let log = MarketOrderLog::new(42, 0);
        assert_eq!(log.market_id, 42);
        assert_eq!(log.page, 0);
        assert_eq!(log.entry_count, 0);
        assert_eq!(log.entries.len(), 0);
    }

    #[test]
    fn test_append_entry() {
        let mut log = MarketOrderLog::new(42, 0);
        let user = Pubkey::new_unique();
        let order_details = OrderDetails {
            order_id: 12345,
            side: OrderSide::Bid,
            qty: 100,
            filled_qty: 0,
            price: OrderPrice::Limit(50000),
            origin: OrderOriginator::User(),
            entry_cond: TriggerCondition::Off(),
            entry_cond_size: TriggerEntrySize::PositionSizePercent(0),
            cancel_cond: TriggerCondition::Off(),
            cancel_cond_2: TriggerCondition::Off(),
            cancel_cond_3: TriggerCondition::Off(),
            tombstone: OrderTombstone::Open(),
            event_history: Default::default(),
            builder_tag: Default::default(),
            _pad1: [0; 64],
            _pad2: [0; 32],
            _pad3: [0; 24],
        };

        log.append_entry(
            user,
            12345,
            OrderUpdateType::OrderEntry,
            order_details,
            None,
            100000,
        )
        .unwrap();

        assert_eq!(log.entry_count, 1);
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].user, user);
        assert_eq!(log.entries[0].order_id, 12345);
        assert_eq!(log.entries[0].update_type, OrderUpdateType::OrderEntry);
    }

    #[test]
    fn test_get_stats() {
        let mut log = MarketOrderLog::new(42, 0);
        let user = Pubkey::new_unique();
        let order_details = OrderDetails::default();

        // Add various types of entries
        log.append_entry(
            user,
            1,
            OrderUpdateType::OrderEntry,
            order_details.clone(),
            None,
            100000,
        )
        .unwrap();
        log.append_entry(
            user,
            1,
            OrderUpdateType::Fill,
            order_details.clone(),
            None,
            100001,
        )
        .unwrap();
        log.append_entry(
            user,
            1,
            OrderUpdateType::Cancel,
            order_details.clone(),
            None,
            100002,
        )
        .unwrap();
        log.append_entry(
            user,
            2,
            OrderUpdateType::OrderEntry,
            order_details.clone(),
            None,
            100003,
        )
        .unwrap();
        log.append_entry(
            user,
            2,
            OrderUpdateType::Liquidation,
            order_details.clone(),
            None,
            100004,
        )
        .unwrap();

        let stats = log.get_stats();
        assert_eq!(stats.total_entries, 5);
        assert_eq!(stats.order_entries, 2);
        assert_eq!(stats.fills, 1);
        assert_eq!(stats.cancels, 1);
        assert_eq!(stats.liquidations, 1);
        assert_eq!(stats.close_positions, 0);
    }

    #[test]
    fn test_append_entry_with_fill_details() {
        let mut log = MarketOrderLog::new(42, 0);
        let user = Pubkey::new_unique();
        let order_details = OrderDetails::default();

        // Create custom fill details
        let fill_details = FillLogDetails {
            price: 50000,
            qty: 100,
            account: CmaFillResult {
                new_net_position: 100,
                old_net_position: 0,
                realized_pnl_banked: 1000,
            },
        };

        // Append entry with fill details
        log.append_entry(
            user,
            12345,
            OrderUpdateType::Fill,
            order_details.clone(),
            Some(fill_details.clone()),
            100000,
        )
        .unwrap();

        // Verify the fill details were correctly recorded
        assert_eq!(log.entry_count, 1);
        assert_eq!(log.entries.len(), 1);
        assert_eq!(log.entries[0].update_type, OrderUpdateType::Fill);
        assert_eq!(log.entries[0].fill_details.price, fill_details.price);
        assert_eq!(log.entries[0].fill_details.qty, fill_details.qty);
        assert_eq!(
            log.entries[0].fill_details.account.new_net_position,
            fill_details.account.new_net_position
        );
        assert_eq!(
            log.entries[0].fill_details.account.old_net_position,
            fill_details.account.old_net_position
        );
        assert_eq!(
            log.entries[0].fill_details.account.realized_pnl_banked,
            fill_details.account.realized_pnl_banked
        );

        // Verify that None fill details uses the empty default
        log.append_entry(
            user,
            12346,
            OrderUpdateType::OrderEntry,
            order_details.clone(),
            None,
            100001,
        )
        .unwrap();

        assert_eq!(log.entry_count, 2);
        assert_eq!(log.entries[1].fill_details.price, EMPTY_FILL_DETAILS.price);
        assert_eq!(log.entries[1].fill_details.qty, EMPTY_FILL_DETAILS.qty);
    }

    #[test]
    fn test_append_user_collateral_update() {
        let mut log = MarketOrderLog::new(42, 0);
        let user = Pubkey::new_unique();
        let collateral_snapshot = 1_000_000_000; // 10 USDC
        let slot = 123456;

        // Append user collateral update
        log.append_user_collateral_update(user, collateral_snapshot, slot)
            .unwrap();

        // Verify the entry was correctly created
        assert_eq!(log.entry_count, 1);
        assert_eq!(log.entries.len(), 1);

        let entry = &log.entries[0];
        assert_eq!(entry.user, user);
        assert_eq!(entry.update_type, OrderUpdateType::UserCollateralUpdate);
        assert_eq!(entry.slot, slot);
        assert_eq!(entry.order_id, 0); // Should be 0 for collateral updates
        assert_eq!(entry.fill_details.qty, collateral_snapshot); // Collateral stored in qty field
        assert_eq!(entry.fill_details.price, 0); // Should be 0
        assert_eq!(entry.order_details, OrderDetails::default()); // Should be default
    }

    #[test]
    fn test_synth_user_collateral_update() {
        let user = Pubkey::new_unique();
        let collateral_snapshot = 500_000_000; // 5 USDC
        let slot = 789012;

        let entry = OrderLogEntry::synth_user_collateral_update(user, collateral_snapshot, slot);

        assert_eq!(entry.user, user);
        assert_eq!(entry.update_type, OrderUpdateType::UserCollateralUpdate);
        assert_eq!(entry.slot, slot);
        assert_eq!(entry.order_id, 0);
        assert_eq!(entry.fill_details.qty, collateral_snapshot);
        assert_eq!(entry.fill_details.price, 0);
        assert_eq!(entry.fill_details.account, CmaFillResult::default());
        assert_eq!(entry.order_details, OrderDetails::default());
    }

    #[test]
    fn test_get_stats_with_collateral_updates() {
        let mut log = MarketOrderLog::new(42, 0);
        let user = Pubkey::new_unique();
        let order_details = OrderDetails::default();

        // Add various types of entries including collateral updates
        log.append_entry(
            user,
            1,
            OrderUpdateType::OrderEntry,
            order_details.clone(),
            None,
            100000,
        )
        .unwrap();
        log.append_user_collateral_update(user, 1_000_000_000, 100001)
            .unwrap();
        log.append_entry(
            user,
            1,
            OrderUpdateType::Fill,
            order_details.clone(),
            None,
            100002,
        )
        .unwrap();
        log.append_user_collateral_update(user, 2_000_000_000, 100003)
            .unwrap();
        log.append_entry(
            user,
            1,
            OrderUpdateType::Cancel,
            order_details.clone(),
            None,
            100004,
        )
        .unwrap();

        let stats = log.get_stats();
        assert_eq!(stats.total_entries, 5);
        assert_eq!(stats.order_entries, 1);
        assert_eq!(stats.fills, 1);
        assert_eq!(stats.cancels, 1);
        assert_eq!(stats.user_collateral_updates, 2);
        assert_eq!(stats.liquidations, 0);
        assert_eq!(stats.close_positions, 0);
    }
}
