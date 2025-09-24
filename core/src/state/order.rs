use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::msg;
use solana_program::pubkey::Pubkey;

/// Order side enumeration
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq)]
pub enum OrderSide {
    Bid = 0,
    Ask = 1,
}

/// Stored in OrderRegistry PDA: ["orders", market_id(8), bump]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct OrderMarker {
    pub user: Pubkey,       // 32 B
    pub order_id: u64,      // 8 B unique per user
    pub order_version: u16, // 2 B
    pub order_page: u32,    // 2 B
    pub _pad1: [u8; 2],     // 16 B padding
}

impl Default for OrderMarker {
    fn default() -> Self {
        Self {
            user: Pubkey::default(),
            order_id: 0,
            order_version: 1,
            order_page: 0,
            _pad1: [0; 2],
        }
    }
}

impl OrderMarker {
    pub fn new(user: Pubkey, order_id: u64) -> Self {
        Self {
            user,
            order_id,
            ..Default::default()
        }
    }
}

/// Stored in user specific PDA: ["orders", market_id(8), user(32), order_id(64), bump]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct OrderDetails {
    pub order_id: u64, // user-provided order ID
    pub side: OrderSide,
    pub qty: u64,        // base lots (1e-6)
    pub filled_qty: u64, // filled qty (1e-6)
    pub price: OrderPrice,
    pub origin: OrderOriginator,
    pub entry_cond: TriggerCondition,
    pub entry_cond_size: TriggerEntrySize,
    pub cancel_cond: TriggerCondition,
    pub cancel_cond_2: TriggerCondition,
    pub cancel_cond_3: TriggerCondition,
    pub tombstone: OrderTombstone,
    pub event_history: EventHistory,
    pub builder_tag: BuilderTag,
    pub _pad1: [u8; 64], // padding
    pub _pad2: [u8; 32], // padding
    pub _pad3: [u8; 24], // padding (total 120 bytes)
}

impl Default for OrderDetails {
    fn default() -> Self {
        Self {
            order_id: 0,
            side: OrderSide::Bid,
            qty: 0,
            filled_qty: 0,
            price: OrderPrice::Market(),
            origin: OrderOriginator::User(),
            entry_cond: TriggerCondition::Off(),
            entry_cond_size: TriggerEntrySize::PositionSizePercent(0),
            cancel_cond: TriggerCondition::Off(),
            cancel_cond_2: TriggerCondition::Off(),
            cancel_cond_3: TriggerCondition::Off(),
            tombstone: OrderTombstone::Empty(),
            event_history: EventHistory::default(),
            builder_tag: BuilderTag::default(),
            _pad1: [0; 64],
            _pad2: [0; 32],
            _pad3: [0; 24],
        }
    }
}

impl OrderDetails {
    /// Create a new OrderDetails with business logic for time-in-force
    pub fn new(
        order_id: u64,
        side: OrderSide,
        qty: u64,
        price: OrderPrice,
        tif: TimeInForce,
    ) -> Self {
        // Determine cancel condition based on time-in-force
        let cancel_cond = match tif {
            TimeInForce::GTC => {
                // GTC: Keep the default Off() condition
                TriggerCondition::Off()
            }
            TimeInForce::IOC => {
                // IOC: Cancel if the order becomes unmarketable
                TriggerCondition::ImmediateOrCancelFail()
            }
            TimeInForce::FOK => {
                // FOK: Cancel if not 100% filled immediately
                TriggerCondition::FillOrKillFail()
            }
            TimeInForce::ALO => {
                // ALO: Only place if order rests
                TriggerCondition::AddLiquidityOnlyFail()
            }
            TimeInForce::GTT(expiry_time) => {
                // GTT: Cancel when the specified time is reached
                TriggerCondition::Time(expiry_time)
            }
        };

        Self {
            order_id,
            side,
            qty,
            filled_qty: 0,
            price,
            origin: OrderOriginator::User(),
            entry_cond: TriggerCondition::Off(),
            entry_cond_size: TriggerEntrySize::PositionSizePercent(0),
            cancel_cond,
            cancel_cond_2: TriggerCondition::Off(),
            cancel_cond_3: TriggerCondition::Off(),
            tombstone: OrderTombstone::Open(),
            event_history: EventHistory::default(),
            builder_tag: BuilderTag::default(),
            _pad1: [0; 64],
            _pad2: [0; 32],
            _pad3: [0; 24],
        }
    }

    /// Calculate unfilled quantity
    pub fn unfilled_qty(&self) -> u64 {
        self.qty.saturating_sub(self.filled_qty)
    }

    /// Process order cancellation, returning unfilled quantity and side
    pub fn process_cancellation(
        &mut self,
        new_tombstone: &OrderTombstone,
        timestamp: i64,
    ) -> Result<(u64, OrderSide), &'static str> {
        // Validate order is alive
        if !self.tombstone.is_alive() {
            msg!("Order is already dead, cannot cancel");
            return Err("Order is already dead");
        }

        // Validate new tombstone is a cancellation state
        if !new_tombstone.is_valid_cancellation() {
            msg!("Invalid cancellation tombstone: {:?}", new_tombstone);
            return Err("Invalid cancellation tombstone");
        }

        self.tombstone = new_tombstone.clone();
        self.event_history.dead_time = timestamp;

        Ok((self.unfilled_qty(), self.side))
    }

    /// Process a fill on this order
    pub fn process_fill(
        &mut self,
        fill_qty: u64,
        fill_price: u64,
        unix_timestamp: i64,
    ) -> Result<OrderFillResult, &'static str> {
        // Validate order is alive
        if !self.tombstone.is_alive() {
            return Err("Cannot fill a dead order");
        }

        // Check that fill doesn't exceed remaining quantity
        let remaining_qty = self.unfilled_qty();
        if fill_qty > remaining_qty {
            return Err("Fill quantity exceeds remaining order quantity");
        }

        // Calculate the weighted average fill price
        let new_filled_qty = self.filled_qty + fill_qty;
        let weighted_avg_price = if self.filled_qty == 0 {
            // First fill - just use the fill price
            fill_price
        } else {
            let new_value = fill_qty as u128 * fill_price as u128;
            let old_value = self.filled_qty as u128 * self.event_history.avg_fill_price as u128;
            let total_value = new_value + old_value;
            (total_value / new_filled_qty as u128) as u64
        };

        // Update filled quantity
        self.filled_qty = new_filled_qty;
        self.event_history.avg_fill_price = weighted_avg_price;

        // Check if order is fully filled
        let is_fully_filled = self.filled_qty >= self.qty;
        if is_fully_filled {
            self.tombstone = OrderTombstone::Filled();
        }

        if self.event_history.first_fill_time == 0 {
            self.event_history.first_fill_time = unix_timestamp;
        }
        self.event_history.last_fill_time = unix_timestamp;

        Ok(OrderFillResult {
            filled_qty: fill_qty,
            weighted_avg_price,
            is_fully_filled,
            side: self.side,
        })
    }
}

/// Result of processing an order fill
#[derive(Debug, Clone, PartialEq)]
pub struct OrderFillResult {
    pub filled_qty: u64,
    pub weighted_avg_price: u64,
    pub is_fully_filled: bool,
    pub side: OrderSide,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Default, PartialEq)]
pub struct EventHistory {
    pub entry_time: i64,
    pub trigger_time: i64,
    pub first_fill_time: i64,
    pub last_fill_time: i64,
    pub did_rest: bool,
    pub dead_time: i64,

    pub take_volume: u64,
    pub priced_make_volume: u64,
    pub pegged_make_volume: u64,
    pub improve_volume: u64,

    pub avg_fill_price: u64,
    pub fees_booked: u64,

    pub _pad1: [u8; 32],
    pub _pad2: [u8; 32],
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Default, PartialEq)]
pub struct BuilderTag {
    pub builder_id: u64,
    pub referrer_id: u64,
    pub _pad1: [u8; 32],
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum OrderTombstone {
    Empty(),
    PreTrigger(),
    Open(),
    Filled(),
    UserCancel(),
    TriggerCancelCond1(),
    TriggerCancelCond2(),
    TriggerCancelCond3(),
    LiquidatorMargin(),
    AutoDeleverage(),
    SystemHalt(),
    Breaker(),
    PositionLimits(),
    MarketClosed(),

    SelfTrade(),
    TickRejected(),
    PriceBandRejected(),
    MinTradeRejected(),
    OpenInterestCap(),
    MaxPositionRejected(),
    MaxOrderSizeRejected(),
    CancelOnEntrySizing(),

    InvalidPrice(),
    InvalidQty(),
    InvalidCond(),
    ForceExpire(),
    Admin(),
    Error(),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum OrderPrice {
    Market(),
    Limit(u64),
    PeggedOffset(i64, PegPriceReference),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum OrderOriginator {
    User(),
    Keeper(),
    OffChainTrigger(),
    Twap(),
    Liquidation(),
    System(),
    Permit(), // Order placed via off-chain permit signature
    VariantPlaceholder([u8; 16]),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum TriggerCondition {
    Off(),
    PriceBelow(u64, PriceReference),
    PriceAbove(u64, PriceReference),
    OrderCancel(u64),
    OrderFill(u64),
    OrderPartialFill(u64, u16),
    ImmediateOrCancelFail(),
    FillOrKillFail(),
    AddLiquidityOnlyFail(),
    ReduceOnlyFail(),
    Time(u64),
    VariantPlaceholder([u8; 16]),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum PriceReference {
    MarkPrice(),
    OraclePrice(),
    SpotPrice(),
    BidPrice(),
    AskPrice(),
    MidPrice(),
    LastTradePrice(),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum PegPriceReference {
    OraclePrice(),
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum TriggerEntrySize {
    PositionSizePercent(u16),
    OrderSizePercent(u16),
    FixedSize(u64),
}

/// Time in Force order types
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Default)]
pub enum TimeInForce {
    /// Immediate or Cancel - execute immediately, cancel remainder
    IOC,
    /// Fill or Kill - execute completely or cancel entirely  
    FOK,
    /// Good Till Cancelled - remain active until cancelled
    #[default]
    GTC,
    /// Add Liquidity Only - only place if order rests
    ALO,
    /// Good Till Time - remain active until specified timestamp
    GTT(u64),
}

impl OrderTombstone {
    /// Check if the tombstone represents an alive order
    pub fn is_alive(&self) -> bool {
        matches!(
            self,
            OrderTombstone::Empty() | OrderTombstone::PreTrigger() | OrderTombstone::Open()
        )
    }

    /// Check if the tombstone represents a valid cancellation state
    pub fn is_valid_cancellation(&self) -> bool {
        !self.is_alive()
    }
}

/// Total number of padding bytes reserved in OrderDetails struct
pub const ORDER_DETAILS_RESERVED_PADDING: usize = 64 + 32 + 24;

/// Compute the maximum possible Borsh-serialized size of OrderDetails using the largest variant payloads
pub fn max_order_details_borsh_size() -> usize {
    let fat = OrderDetails {
        order_id: u64::MAX,
        side: OrderSide::Ask,
        qty: u64::MAX,
        filled_qty: u64::MAX,
        price: OrderPrice::PeggedOffset(i64::MAX, PegPriceReference::OraclePrice()),
        origin: OrderOriginator::VariantPlaceholder([0xff; 16]),
        entry_cond: TriggerCondition::OrderPartialFill(u64::MAX, u16::MAX),
        entry_cond_size: TriggerEntrySize::FixedSize(u64::MAX),
        cancel_cond: TriggerCondition::OrderPartialFill(u64::MAX, u16::MAX),
        cancel_cond_2: TriggerCondition::OrderPartialFill(u64::MAX, u16::MAX),
        cancel_cond_3: TriggerCondition::OrderPartialFill(u64::MAX, u16::MAX),
        tombstone: OrderTombstone::Error(),
        event_history: EventHistory::default(),
        builder_tag: BuilderTag::default(),
        _pad1: [0u8; 64],
        _pad2: [0u8; 32],
        _pad3: [0u8; 24],
    };
    fat.try_to_vec()
        .expect("Borsh should serialize OrderDetails")
        .len()
}

#[cfg(test)]
mod padding_tests {
    use super::*;
    #[test]
    fn padding_covers_max_payload() {
        let default_len = OrderDetails::default().try_to_vec().unwrap().len();
        let max_len = max_order_details_borsh_size();
        assert!(
            max_len - default_len <= ORDER_DETAILS_RESERVED_PADDING,
            "Reserved padding ({}) should cover max extra payload ({}).",
            ORDER_DETAILS_RESERVED_PADDING,
            (max_len - default_len)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_order_marker_new() {
        let user = Pubkey::new_unique();
        let order_id = 12345;

        let marker = OrderMarker::new(user, order_id);

        assert_eq!(marker.user, user);
        assert_eq!(marker.order_id, order_id);
        assert_eq!(marker.order_version, 1);
        assert_eq!(marker.order_page, 0);
        assert_eq!(marker._pad1, [0; 2]);
    }

    #[test]
    fn test_tombstone_is_alive() {
        // Test alive states
        assert!(OrderTombstone::Empty().is_alive());
        assert!(OrderTombstone::PreTrigger().is_alive());
        assert!(OrderTombstone::Open().is_alive());

        // Test dead states
        assert!(!OrderTombstone::Filled().is_alive());
        assert!(!OrderTombstone::UserCancel().is_alive());
        assert!(!OrderTombstone::TriggerCancelCond1().is_alive());
        assert!(!OrderTombstone::TriggerCancelCond2().is_alive());
        assert!(!OrderTombstone::TriggerCancelCond3().is_alive());
        assert!(!OrderTombstone::LiquidatorMargin().is_alive());
        assert!(!OrderTombstone::AutoDeleverage().is_alive());
        assert!(!OrderTombstone::SystemHalt().is_alive());
        assert!(!OrderTombstone::Breaker().is_alive());
        assert!(!OrderTombstone::PositionLimits().is_alive());
        assert!(!OrderTombstone::MarketClosed().is_alive());
        assert!(!OrderTombstone::SelfTrade().is_alive());
        assert!(!OrderTombstone::TickRejected().is_alive());
        assert!(!OrderTombstone::PriceBandRejected().is_alive());
        assert!(!OrderTombstone::MinTradeRejected().is_alive());
        assert!(!OrderTombstone::OpenInterestCap().is_alive());
        assert!(!OrderTombstone::MaxPositionRejected().is_alive());
        assert!(!OrderTombstone::MaxOrderSizeRejected().is_alive());
        assert!(!OrderTombstone::InvalidPrice().is_alive());
        assert!(!OrderTombstone::InvalidQty().is_alive());
        assert!(!OrderTombstone::InvalidCond().is_alive());
        assert!(!OrderTombstone::ForceExpire().is_alive());
        assert!(!OrderTombstone::Admin().is_alive());
        assert!(!OrderTombstone::Error().is_alive());
    }

    #[test]
    fn test_tombstone_is_valid_cancellation() {
        // Test alive states (not valid for cancellation)
        assert!(!OrderTombstone::Empty().is_valid_cancellation());
        assert!(!OrderTombstone::PreTrigger().is_valid_cancellation());
        assert!(!OrderTombstone::Open().is_valid_cancellation());

        // Test cancellation states
        assert!(OrderTombstone::Filled().is_valid_cancellation());
        assert!(OrderTombstone::UserCancel().is_valid_cancellation());
        assert!(OrderTombstone::LiquidatorMargin().is_valid_cancellation());
        assert!(OrderTombstone::SystemHalt().is_valid_cancellation());
        assert!(OrderTombstone::Admin().is_valid_cancellation());
        assert!(OrderTombstone::ForceExpire().is_valid_cancellation());
    }

    #[test]
    fn test_order_marker_default() {
        let marker = OrderMarker::default();

        assert_eq!(marker.user, Pubkey::default());
        assert_eq!(marker.order_id, 0);
        assert_eq!(marker.order_version, 1);
        assert_eq!(marker.order_page, 0);
        assert_eq!(marker._pad1, [0; 2]);
    }

    #[test]
    fn test_order_details_new_gtc() {
        let order_id = 12345u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000,                  // 1.0 tokens
            OrderPrice::Limit(100_000), // $100
            TimeInForce::GTC,
        );

        assert_eq!(order_details.order_id, order_id);
        assert_eq!(order_details.side, OrderSide::Bid);
        assert_eq!(order_details.qty, 1_000_000);
        assert_eq!(order_details.filled_qty, 0);
        assert_eq!(order_details.price, OrderPrice::Limit(100_000));
        assert_eq!(order_details.cancel_cond, TriggerCondition::Off());
        assert_eq!(order_details.tombstone, OrderTombstone::Open());
    }

    #[test]
    fn test_order_details_new_ioc() {
        let order_id = 22222u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            2_000_000,                  // 2.0 tokens
            OrderPrice::Limit(101_000), // $101
            TimeInForce::IOC,
        );

        assert_eq!(order_details.side, OrderSide::Ask);
        assert_eq!(order_details.qty, 2_000_000);
        assert_eq!(
            order_details.cancel_cond,
            TriggerCondition::ImmediateOrCancelFail()
        );
        assert_eq!(order_details.tombstone, OrderTombstone::Open());
    }

    #[test]
    fn test_order_details_new_fok() {
        let order_id = 33333u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            5_000_000,                 // 5.0 tokens
            OrderPrice::Limit(99_000), // $99
            TimeInForce::FOK,
        );

        assert_eq!(order_details.side, OrderSide::Bid);
        assert_eq!(order_details.qty, 5_000_000);
        assert_eq!(
            order_details.cancel_cond,
            TriggerCondition::FillOrKillFail()
        );
        assert_eq!(order_details.tombstone, OrderTombstone::Open());
    }

    #[test]
    fn test_order_details_new_alo() {
        let order_id = 44444u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            3_000_000,                  // 3.0 tokens
            OrderPrice::Limit(102_000), // $102
            TimeInForce::ALO,
        );

        assert_eq!(order_details.side, OrderSide::Ask);
        assert_eq!(order_details.qty, 3_000_000);
        assert_eq!(
            order_details.cancel_cond,
            TriggerCondition::AddLiquidityOnlyFail()
        );
        assert_eq!(order_details.tombstone, OrderTombstone::Open());
    }

    #[test]
    fn test_order_details_new_gtt() {
        let order_id = 55555u64;
        let expiry_time = 1640995200; // Unix timestamp
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            4_000_000,                 // 4.0 tokens
            OrderPrice::Limit(98_000), // $98
            TimeInForce::GTT(expiry_time),
        );

        assert_eq!(order_details.side, OrderSide::Bid);
        assert_eq!(order_details.qty, 4_000_000);
        assert_eq!(
            order_details.cancel_cond,
            TriggerCondition::Time(expiry_time)
        );
        assert_eq!(order_details.tombstone, OrderTombstone::Open());
    }

    #[test]
    fn test_order_details_new_market_price() {
        let order_id = 66666u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            1_500_000, // 1.5 tokens
            OrderPrice::Market(),
            TimeInForce::IOC, // Market orders typically IOC
        );

        assert_eq!(order_details.side, OrderSide::Ask);
        assert_eq!(order_details.qty, 1_500_000);
        assert_eq!(order_details.price, OrderPrice::Market());
        assert_eq!(
            order_details.cancel_cond,
            TriggerCondition::ImmediateOrCancelFail()
        );
    }

    #[test]
    fn test_order_details_new_pegged_price() {
        let order_id = 77777u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            2_500_000,                                                         // 2.5 tokens
            OrderPrice::PeggedOffset(-1000, PegPriceReference::OraclePrice()), // $1 below oracle
            TimeInForce::GTC,
        );

        assert_eq!(order_details.side, OrderSide::Bid);
        assert_eq!(order_details.qty, 2_500_000);
        assert_eq!(
            order_details.price,
            OrderPrice::PeggedOffset(-1000, PegPriceReference::OraclePrice())
        );
        assert_eq!(order_details.cancel_cond, TriggerCondition::Off());
    }

    #[test]
    fn test_order_details_default_fields() {
        let order_id = 88888u64;
        let order_details = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Check that default fields are set correctly
        assert_eq!(order_details.filled_qty, 0);
        assert_eq!(order_details.entry_cond, TriggerCondition::Off());
        assert_eq!(
            order_details.entry_cond_size,
            TriggerEntrySize::PositionSizePercent(0)
        );
        assert_eq!(order_details.cancel_cond_2, TriggerCondition::Off());
        assert_eq!(order_details.cancel_cond_3, TriggerCondition::Off());
        assert_eq!(order_details.tombstone, OrderTombstone::Open());
        assert_eq!(order_details.event_history, EventHistory::default());
        assert_eq!(order_details.builder_tag, BuilderTag::default());
        assert_eq!(order_details._pad1, [0; 64]);
        assert_eq!(order_details._pad2, [0; 32]);
        assert_eq!(order_details._pad3, [0; 24]);
    }

    #[test]
    fn test_order_details_default() {
        let order_details = OrderDetails::default();

        assert_eq!(order_details.side, OrderSide::Bid);
        assert_eq!(order_details.qty, 0);
        assert_eq!(order_details.filled_qty, 0);
        assert_eq!(order_details.price, OrderPrice::Market());
        assert_eq!(order_details.tombstone, OrderTombstone::Empty());
    }

    #[test]
    fn test_time_in_force_default() {
        let tif = TimeInForce::default();
        assert_eq!(tif, TimeInForce::GTC);
    }

    #[test]
    fn test_order_side_values() {
        assert_eq!(OrderSide::Bid as u8, 0);
        assert_eq!(OrderSide::Ask as u8, 1);
    }

    #[test]
    fn test_order_side_copy_clone() {
        let side1 = OrderSide::Bid;
        let side2 = side1; // Copy
        let side3 = side1; // Clone

        assert_eq!(side1, side2);
        assert_eq!(side1, side3);
        assert_eq!(side2, side3);
    }

    #[test]
    fn test_event_history_default() {
        let history = EventHistory::default();

        assert_eq!(history.entry_time, 0);
        assert_eq!(history.trigger_time, 0);
        assert_eq!(history.first_fill_time, 0);
        assert_eq!(history.last_fill_time, 0);
        assert!(!history.did_rest);
        assert_eq!(history.dead_time, 0);
        assert_eq!(history.take_volume, 0);
        assert_eq!(history.priced_make_volume, 0);
        assert_eq!(history.pegged_make_volume, 0);
        assert_eq!(history.improve_volume, 0);
        assert_eq!(history.avg_fill_price, 0);
        assert_eq!(history.fees_booked, 0);
    }

    #[test]
    fn test_builder_tag_default() {
        let tag = BuilderTag::default();

        assert_eq!(tag.builder_id, 0);
        assert_eq!(tag.referrer_id, 0);
        assert_eq!(tag._pad1, [0; 32]);
    }

    #[test]
    fn test_all_time_in_force_variants() {
        // Test all TimeInForce variants to ensure they compile and work
        let variants = vec![
            TimeInForce::IOC,
            TimeInForce::FOK,
            TimeInForce::GTC,
            TimeInForce::ALO,
            TimeInForce::GTT(1640995200),
        ];

        for (i, tif) in variants.into_iter().enumerate() {
            let order_id = 90000 + i as u64;
            let order_details = OrderDetails::new(
                order_id,
                OrderSide::Bid,
                1_000_000,
                OrderPrice::Limit(100_000),
                tif.clone(),
            );

            // Each variant should set a different cancel condition
            match tif {
                TimeInForce::IOC => assert_eq!(
                    order_details.cancel_cond,
                    TriggerCondition::ImmediateOrCancelFail()
                ),
                TimeInForce::FOK => assert_eq!(
                    order_details.cancel_cond,
                    TriggerCondition::FillOrKillFail()
                ),
                TimeInForce::GTC => assert_eq!(order_details.cancel_cond, TriggerCondition::Off()),
                TimeInForce::ALO => assert_eq!(
                    order_details.cancel_cond,
                    TriggerCondition::AddLiquidityOnlyFail()
                ),
                TimeInForce::GTT(time) => {
                    assert_eq!(order_details.cancel_cond, TriggerCondition::Time(time))
                }
            }
        }
    }

    #[test]
    fn test_order_details_unfilled_qty() {
        let order_id = 99999u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000, // 1.0 tokens
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Test fully unfilled order
        assert_eq!(order.unfilled_qty(), 1_000_000);

        // Test partially filled order
        order.filled_qty = 300_000;
        assert_eq!(order.unfilled_qty(), 700_000);

        // Test fully filled order
        order.filled_qty = 1_000_000;
        assert_eq!(order.unfilled_qty(), 0);

        // Test overfilled order (should saturate to 0)
        order.filled_qty = 1_200_000;
        assert_eq!(order.unfilled_qty(), 0);
    }

    #[test]
    fn test_order_details_process_cancellation_success() {
        let order_id = 111111u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        let mut order2 = order.clone();

        // Test successful cancellation with UserCancel
        let result = order.process_cancellation(&OrderTombstone::UserCancel(), 0);
        assert!(result.is_ok());
        let (unfilled_qty, side) = result.unwrap();
        assert_eq!(unfilled_qty, 1_000_000);
        assert_eq!(side, OrderSide::Ask);

        // Test with partially filled order
        order2.filled_qty = 400_000;
        let result2 = order2.process_cancellation(&OrderTombstone::Admin(), 0);
        assert!(result2.is_ok());
        let (unfilled_qty2, side2) = result2.unwrap();
        assert_eq!(unfilled_qty2, 600_000);
        assert_eq!(side2, OrderSide::Ask);
    }

    #[test]
    fn test_order_details_process_cancellation_already_dead() {
        let order_id = 222222u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Set order as already cancelled
        order.tombstone = OrderTombstone::UserCancel();

        let result = order.process_cancellation(&OrderTombstone::Admin(), 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Order is already dead");

        // Test with filled order
        order.tombstone = OrderTombstone::Filled();
        let result2 = order.process_cancellation(&OrderTombstone::UserCancel(), 0);
        assert!(result2.is_err());
        assert_eq!(result2.unwrap_err(), "Order is already dead");
    }

    #[test]
    fn test_order_details_process_cancellation_invalid_tombstone() {
        let order_id = 333333u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Try to cancel with alive tombstone states
        let result = order.process_cancellation(&OrderTombstone::Open(), 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid cancellation tombstone");

        let result2 = order.process_cancellation(&OrderTombstone::PreTrigger(), 0);
        assert!(result2.is_err());
        assert_eq!(result2.unwrap_err(), "Invalid cancellation tombstone");

        let result3 = order.process_cancellation(&OrderTombstone::Empty(), 0);
        assert!(result3.is_err());
        assert_eq!(result3.unwrap_err(), "Invalid cancellation tombstone");
    }

    #[test]
    fn test_order_details_process_fill_first_fill() {
        let order_id = 444444u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Process first fill
        let result = order.process_fill(300_000, 99_500, 0);
        assert!(result.is_ok());

        let fill_result = result.unwrap();
        assert_eq!(fill_result.filled_qty, 300_000);
        assert_eq!(fill_result.weighted_avg_price, 99_500);
        assert!(!fill_result.is_fully_filled);
        assert_eq!(fill_result.side, OrderSide::Bid);

        // Check order state
        assert_eq!(order.filled_qty, 300_000);
        assert_eq!(order.event_history.avg_fill_price, 99_500);
        assert_eq!(order.price, OrderPrice::Limit(100_000));
        assert_eq!(order.qty, 1_000_000);
        assert_eq!(order.tombstone, OrderTombstone::Open());
    }

    #[test]
    fn test_order_details_process_fill_multiple_fills() {
        let order_id = 555555u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // First fill
        let result1 = order.process_fill(400_000, 100_200, 0);
        assert!(result1.is_ok());
        assert_eq!(order.filled_qty, 400_000);
        assert_eq!(order.event_history.avg_fill_price, 100_200);

        // Second fill - should calculate weighted average
        let result2 = order.process_fill(200_000, 100_500, 0);
        assert!(result2.is_ok());

        let fill_result = result2.unwrap();
        assert_eq!(fill_result.filled_qty, 200_000);
        // Weighted avg: (400_000 * 100_200 + 200_000 * 100_500) / 600_000 = 100_300
        assert_eq!(order.event_history.avg_fill_price, 100_300);
        assert_eq!(fill_result.weighted_avg_price, 100_300);
        assert!(!fill_result.is_fully_filled);

        assert_eq!(order.filled_qty, 600_000);
        assert_eq!(order.qty, 1_000_000);
        assert_eq!(order.price, OrderPrice::Limit(100_000));
    }

    #[test]
    fn test_order_details_process_fill_fully_filled() {
        let order_id = 666666u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            500_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Fill entire order
        let result = order.process_fill(500_000, 99_800, 0);
        assert!(result.is_ok());

        let fill_result = result.unwrap();
        assert_eq!(fill_result.filled_qty, 500_000);
        assert_eq!(fill_result.weighted_avg_price, 99_800);
        assert!(fill_result.is_fully_filled);
        assert_eq!(fill_result.side, OrderSide::Bid);

        // Check order state
        assert_eq!(order.filled_qty, 500_000);
        assert_eq!(order.tombstone, OrderTombstone::Filled());
    }

    #[test]
    fn test_order_details_process_fill_overfill() {
        let order_id = 777777u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Fill part of the order first
        order.filled_qty = 700_000;

        // Try to fill more than remaining
        let result = order.process_fill(400_000, 100_100, 0);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err(),
            "Fill quantity exceeds remaining order quantity"
        );

        // Order should be unchanged
        assert_eq!(order.filled_qty, 700_000);
    }

    #[test]
    fn test_order_details_process_fill_dead_order() {
        let order_id = 888888u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Mark order as cancelled
        order.tombstone = OrderTombstone::UserCancel();

        // Try to fill
        let result = order.process_fill(100_000, 99_900, 0);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Cannot fill a dead order");
    }

    #[test]
    fn test_process_fill_sets_event_times() {
        let order_id = 999999u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Bid,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Process first fill with timestamp 1000
        let result1 = order.process_fill(300_000, 99_500, 1000);
        assert!(result1.is_ok());

        // Verify first_fill_time and last_fill_time are set correctly
        assert_eq!(order.event_history.first_fill_time, 1000);
        assert_eq!(order.event_history.last_fill_time, 1000);

        // Process second fill with timestamp 2000
        let result2 = order.process_fill(300_000, 99_600, 2000);
        assert!(result2.is_ok());

        // Verify first_fill_time remains unchanged and last_fill_time is updated
        assert_eq!(order.event_history.first_fill_time, 1000);
        assert_eq!(order.event_history.last_fill_time, 2000);

        // Process final fill with timestamp 3000
        let result3 = order.process_fill(400_000, 99_700, 3000);
        assert!(result3.is_ok());

        // Verify first_fill_time still remains unchanged and last_fill_time is updated again
        assert_eq!(order.event_history.first_fill_time, 1000);
        assert_eq!(order.event_history.last_fill_time, 3000);
    }

    #[test]
    fn test_process_cancellation_sets_dead_time() {
        let order_id = 888888u64;
        let mut order = OrderDetails::new(
            order_id,
            OrderSide::Ask,
            1_000_000,
            OrderPrice::Limit(100_000),
            TimeInForce::GTC,
        );

        // Set entry time for completeness
        order.event_history.entry_time = 500;

        // Cancel order with timestamp 1500
        let result = order.process_cancellation(&OrderTombstone::UserCancel(), 1500);
        assert!(result.is_ok());

        // Verify dead_time is set correctly
        assert_eq!(order.event_history.dead_time, 1500);

        // Verify tombstone is updated
        assert_eq!(order.tombstone, OrderTombstone::UserCancel());
    }
}
