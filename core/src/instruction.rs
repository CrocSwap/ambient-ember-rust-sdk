use crate::{OrderOriginator, OrderTombstone, TimeInForce};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Instruction enum for the Ember testnet program.
#[derive(BorshSerialize, BorshDeserialize, Debug)]
#[repr(u8)]
pub enum TestnetIx {
    /// Initialize the global state with admins, keepers, defaults and withdraw guard.
    InitializeGlobal {
        /// Minimum deposit size in lamports
        min_deposit_size: u64,
    },
    /// Initialize a new market with parameters.
    InitializeMarket {
        market_id: u64,
        oracle: Pubkey,
        tick_size: u64,
        im_bps: u16,
        mm_bps: u16,
        base_token: Pubkey,
        min_order_size: u64,
        max_order_size: Option<u64>,
        max_oi_size: Option<u64>,
        max_user_oi_size: Option<u64>,
        fill_offset: Option<u16>,
    },
    /// Deposit collateral into CMA.
    Deposit { amount: u64 },
    /// Withdraw collateral from CMA.
    Withdraw { amount: u64 },
    /// Commit collateral.
    CommitCollateral { market_id: u64, amount: u64 },
    /// Uncommit collateral.
    UncommitCollateral { market_id: u64, amount: u64 },
    /// Run keeper cycle: match and liquidate.
    KeeperCycle { bid: u64, ask: u64 },
    /// Add a new admin.
    AddAdmin { target: Pubkey },
    /// Remove an existing admin.
    RemoveAdmin { target: Pubkey },
    /// Add a new keeper.
    AddKeeper { target: Pubkey },
    /// Remove an existing keeper.
    RemoveKeeper { target: Pubkey },
    /// Ping instruction to emit a Pong log with the caller and amount.
    /// Accounts: Caller account (signer).
    Ping {
        /// The ping value to echo.
        amount: u64,
    },
    /// Initialize a Cross-Margin Account (CMA) PDA for a user.
    /// Accounts: Actor (signer), Target, RentPayer (signer), Global, CMA PDA, System program
    InitCma,
    /// Initialize OrderDetails PDA for a user in a specific market.
    /// Accounts: Actor (signer), RentPayer (signer), OrderDetails PDA, Global, System program
    InitOrderDetails {
        /// The market ID this order details belongs to
        market_id: u64,
        /// The user for whom to create the OrderDetails
        user: Pubkey,
        /// The page offset (usually 0 for initial page)
        page_offset: u32,
    },
    /// Liquidate a position by creating and immediately filling an order (keeper only)
    /// Accounts: Keeper (signer), User, Global, CMA, Market, OrderDetails
    LiqPosition {
        /// The market ID for the liquidation
        market_id: u64,
        /// The user whose position is being liquidated
        user: Pubkey,
        /// Unique order ID for this liquidation
        order_id: u64,
        /// Order side (Bid/Ask) to reduce the position
        side: u8,
        /// Quantity to liquidate in base lots
        qty: u64,
        /// Liquidation price
        price: u64,
    },
    /// Close position entry - automatically closes user's net position
    /// Acts similar to OrderEntry but automatically sets quantity to net position
    /// Skips minimum size check
    /// Accounts: Actor (signer), Target, Global, CMA, Market, OrderDetails
    ClosePositionEntry {
        /// The market ID to close position in
        market_id: u64,
        /// User-supplied order ID (unique per user)
        order_id: u64,
        /// Limit price
        price: u64,
        /// Time in force
        tif: TimeInForce,
    },
    /// Update market data with latest price values (keeper only)
    /// Accounts: Keeper (signer), Target (dummy - same as keeper), Global, Market
    UpdateMarketData {
        /// The market ID to update
        market_id: u64,
        /// Latest bid price
        last_bid: u64,
        /// Latest ask price  
        last_ask: u64,
        /// Latest trade price
        last_trade_price: u64,
        /// Latest mark price
        last_mark_price: u64,
    },
    /// Set user-specific margin requirement for a market
    /// Accounts: Actor (signer), Target, Global, CMA, Market
    SetUserMargin {
        /// The market ID to set margin for
        market_id: u64,
        /// The token mint for the margin bucket
        token: Pubkey,
        /// User-specified initial margin in basis points (100 = 1%)
        user_set_im_bps: u16,
    },
    /// Reallocate OrderDetails PDA to accommodate more orders
    /// Accounts: Actor (signer), Target (dummy), Global, OrderDetails, RentPayer (signer), System program
    ReallocOrderDetails {
        /// The market ID for the OrderDetails PDA
        market_id: u64,
        /// The user who owns the OrderDetails PDA
        user: Pubkey,
        /// Additional capacity to allocate (in number of orders)
        additional_capacity: u32,
    },
    // InitMarketOrderLog removed - consolidated into InitializeMarket
    /// Reallocate market order log to accommodate more entries
    /// Always expands by exactly 10KB per instruction due to Solana limits
    /// Accounts: Actor (signer), Global, Market, MarketOrderLog, RentPayer (signer), System program
    ReallocMarketOrderLog {
        /// The market ID for the log
        market_id: u64,
        /// The page number to reallocate
        page: u32,
        /// Additional capacity to allocate (in number of entries)
        additional_capacity: usize,
    },
    /// Place an order using the dual storage system
    /// Accounts: Actor (signer), Target, Global, CMA, Market, PerOrderPDA, MarketOrderLog, RentPayer (signer), System program
    OrderEntry {
        /// The market ID to place the order in
        market_id: u64,
        /// User-supplied order ID (unique per user)
        order_id: u64,
        /// Order side: 0 = Bid, 1 = Ask
        side: u8,
        /// Order quantity in base lots
        qty: u64,
        /// Limit price (ignored for market orders)
        price: u64,
        /// Time in force variant
        tif: TimeInForce,
        /// Order originator
        origin: OrderOriginator,
    },
    /// Cancel an order using the dual storage system
    /// Accounts: Actor (signer), Target, Global, CMA, Market, PerOrderPDA, MarketOrderLog
    CancelOrder {
        /// The market ID
        market_id: u64,
        /// The order ID to cancel
        order_id: u64,
        /// The tombstone to set (determines user vs system mode)
        tombstone: OrderTombstone,
    },
    /// Fill an order using the dual storage system (keeper only)
    /// Accounts: Keeper (signer), Target, Global, CMA, Market, PerOrderPDA, MarketOrderLog
    FillOrder {
        /// Market ID
        market_id: u64,
        /// User who owns the order
        user: Pubkey,
        /// Order ID to fill
        order_id: u64,
        /// Fill quantity
        fill_qty: u64,
        /// Fill price
        fill_price: u64,
    },
    /// Fill an order using a keeper-signed quote (anyone can submit)
    /// Accounts: Submitter (signer), InstructionsSysvar, Global, CMA, Market, PerOrderPDA, MarketOrderLog
    /// Requires instruction 0 to be an ed25519 signature verification of the quote by an authorized keeper
    FillOrderQuote {
        /// Serialized OffchainFillQuote data
        quote_bytes: Vec<u8>,
        /// Public key of the keeper who signed this quote
        keeper_pubkey: Pubkey,
    },
    /// Reset clearing house stats for a market (admin only)
    /// Resets open interest, clearing position, entry price, and realized PnL to zero
    /// Accounts: Admin (signer), Target (dummy - same as admin), Global, Market
    ResetClearingHouse {
        /// The market ID to reset
        market_id: u64,
    },
    /// Snapshot a user's current collateral into the market order log
    /// This is used to get pre-existing collateral into the log for liquidation processing
    /// Accounts: Actor (signer), User, Global, CMA, Market, MarketOrderLog
    SnapshotCollateral {
        /// The market ID for the collateral snapshot
        market_id: u64,
        /// The user whose collateral to snapshot
        user: Pubkey,
    },
    /// Fill an order at market price based on last bid/ask plus fill offset (keeper only)
    /// Accounts: Keeper (signer), Target, Global, CMA, Market, PerOrderPDA, MarketOrderLog
    FillAtMarket {
        /// Market ID
        market_id: u64,
        /// User who owns the order
        user: Pubkey,
        /// Order ID to fill
        order_id: u64,
    },
    /// Reallocate market order log to accommodate more entries (v2 with buffer target)
    /// Only reallocates if current free space is less than buffer_target
    /// Accounts: Actor (signer), Global, Market, MarketOrderLog, RentPayer (signer), System program
    ReallocMarketOrderLogV2 {
        /// The market ID for the log
        market_id: u64,
        /// The page number to reallocate
        page: u32,
        /// Additional capacity to allocate (in number of entries)
        additional_capacity: usize,
        /// Optional buffer target in bytes - if set, only realloc if free space < buffer_target
        buffer_target: Option<usize>,
    },
    /// Increment the current log page for a market when the current page is near capacity
    /// This atomically updates market.current_log_page and initializes the new page
    /// Accounts: Keeper (signer), Global, Market, CurrentMarketOrderLog, NewMarketOrderLog, RentPayer (signer), System program
    IncrementLogPage {
        /// The market ID to increment the page for
        market_id: u64,
    },
    /// Place an order using the dual storage system (V2)
    /// Accounts: Actor (signer), Target, Global, CMA, Market, PerOrderPDA, MarketOrderLog, RentPayer (signer), System program
    OrderEntryV2 {
        /// The market ID to place the order in
        market_id: u64,
        /// User-supplied order ID (unique per user)
        order_id: u64,
        /// Order side: 0 = Bid, 1 = Ask
        side: u8,
        /// Order quantity in base lots
        qty: u64,
        /// Limit price (ignored for market orders)
        price: Option<u64>,
        /// Time in force variant
        tif: TimeInForce,
        /// Order originator
        origin: OrderOriginator,
        /// Reduce only
        reduce_only: bool,
        /// Optional trigger price (not yet supported; must be None)
        trigger_price: Option<u64>,
        /// Trigger type (not yet supported; must be 0)
        trigger_type: u8,
        /// Price peg type (not yet supported; must be 0)
        price_peg_type: u8,
        /// Builder tag code (will be set into OrderDetails.builder_tag.builder_id)
        builder_code: Option<u16>,
    },
    /// Consume a signed permit envelope to execute an action
    /// Accounts: Submitter (signer), InstructionsSysvar, Global, (action-specific accounts)
    ConsumePermit {
        /// Borsh-encoded PermitEnvelopeV1
        permit_bytes: Vec<u8>,
        /// Index of the ed25519/secp256k1 verify instruction in this transaction
        verify_ix_index: u8,
    },
    /// Delegate session permissions to another key (owner-signed)
    /// Accounts: Owner (signer), SessionPda, System program
    DelegateSession {
        /// Session public key to delegate to
        session: Pubkey,
        /// Expiry timestamp (Unix seconds)
        expires_unix: i64,
        /// Bitset of allowed scopes (place, cancel, withdraw, set_leverage)
        scopes_bits: u32,
        /// 24-hour withdrawal limit in native units
        withdraw_limit_24h: u64,
        /// Per-market size limit in lots
        per_market_size_limit_lots: i64,
    },
    /// Revoke a session delegation (owner-signed)
    /// Accounts: Owner (signer), SessionPda
    RevokeSession {
        /// Session public key to revoke
        session: Pubkey,
    },
    /// Create an allowance for limited-use permits (owner-signed)
    /// Accounts: Owner (signer), AllowancePda, System program
    CreateAllowance {
        /// Unique allowance ID
        id: [u8; 32],
        /// Session key this allowance is for
        session: Pubkey,
        /// Number of uses allowed
        uses: u32,
        /// Expiry timestamp (Unix seconds)
        expires_unix: i64,
        /// Bitset of allowed scopes
        scopes_bits: u32,
    },
    /// Revoke an allowance (owner-signed)
    /// Accounts: Owner (signer), AllowancePda
    RevokeAllowance {
        /// Allowance ID to revoke
        id: [u8; 32],
        /// Session key the allowance was for
        session: Pubkey,
    },
    /// Initialize nonce replay PDAs (nonce window and sequence)
    /// Accounts: Actor (signer, rent payer), Target user, NonceWindow PDA, Sequence PDA, System program
    InitNonceWindow {
        /// Window size (k) for HL-style replay protection
        k: u8,
    },
    /// Credit collateral via testnet-only faucet (keeper only)
    /// Accounts: Keeper (signer), Target user, Global, Market, CMA
    FaucetCredit {
        /// Market to source base mint from
        market_id: u64,
        /// Amount of collateral to credit (in native units)
        amount: u64,
        /// Recipient user public key
        recipient: Pubkey,
    },
    /// Update L2 order book snapshot for a market (keeper only)
    /// Accounts: Keeper (signer), Target (dummy - same as keeper), Global, Market, OrderBookSnapshot
    UpdateOrderBookSnapshot {
        /// The market ID to update
        market_id: u64,
        /// Number of significant figures (Hyperliquid-compatible, 2-5)
        n_sig_figs: u8,
        /// Bid levels ordered best-to-worst (descending price)
        bids: Vec<OrderBookLevelInput>,
        /// Ask levels ordered best-to-worst (ascending price)
        asks: Vec<OrderBookLevelInput>,
    },
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Eq)]
pub struct OrderBookLevelInput {
    /// Price expressed in 1e-6 precision like other on-chain prices
    pub price: i64,
    /// Aggregate size at this level in 1e-8 precision
    pub size: u64,
    /// Number of constituent orders (capped to u16 on-chain)
    pub order_count: u16,
}

impl OrderBookLevelInput {
    pub fn is_length_valid(len: usize) -> bool {
        len <= crate::state::orderbook::ORDERBOOK_LEVELS_PER_SIDE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use borsh::ser::BorshSerialize;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_initialize_global_discriminant() {
        let ix = TestnetIx::InitializeGlobal {
            min_deposit_size: 1000000,
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 0);
    }

    #[test]
    fn test_initialize_market_discriminant() {
        let ix = TestnetIx::InitializeMarket {
            market_id: 7,
            oracle: Pubkey::default(),
            tick_size: 1,
            im_bps: 100,
            mm_bps: 50,
            base_token: Pubkey::default(),
            min_order_size: 1000000,
            max_order_size: None,
            max_oi_size: None,
            max_user_oi_size: None,
            fill_offset: None,
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 1);
    }

    #[test]
    fn test_deposit_discriminant() {
        let ix = TestnetIx::Deposit { amount: 123 };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 2);
    }

    #[test]
    fn test_withdraw_discriminant() {
        let ix = TestnetIx::Withdraw { amount: 456 };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 3);
    }

    #[test]
    fn test_faucet_discriminant() {
        let ix = TestnetIx::FaucetCredit {
            market_id: 64,
            amount: 1_000_000,
            recipient: Pubkey::default(),
        };
        let data = ix.try_to_vec().unwrap();
        // FaucetCredit is appended to the enum; ensure discriminant matches serialization.
        let discr = unsafe { *(std::ptr::addr_of!(ix) as *const u8) };
        assert_eq!(data[0], discr);
    }

    #[test]
    fn test_commit_collateral_discriminant() {
        let ix = TestnetIx::CommitCollateral {
            amount: 789,
            market_id: 0,
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 4);
    }

    #[test]
    fn test_uncommit_collateral_discriminant() {
        let ix = TestnetIx::UncommitCollateral {
            amount: 1011,
            market_id: 0,
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 5);
    }

    #[test]
    fn test_cancel_order_discriminant() {
        let ix = TestnetIx::CancelOrder {
            market_id: 0,
            order_id: 2,
            tombstone: OrderTombstone::UserCancel(),
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 21);
    }

    #[test]
    fn test_orderentry_discriminant() {
        let ix = TestnetIx::OrderEntry {
            market_id: 0,
            order_id: 1,
            side: 0,
            qty: 100,
            price: 100,
            tif: TimeInForce::GTC,
            origin: OrderOriginator::User(),
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 20);
    }

    #[test]
    fn test_keeper_cycle_discriminant() {
        let ix = TestnetIx::KeeperCycle { bid: 10, ask: 20 };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 6);
    }

    #[test]
    fn test_add_admin_discriminant() {
        let ix = TestnetIx::AddAdmin {
            target: Pubkey::default(),
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 7);
    }

    #[test]
    fn test_remove_admin_discriminant() {
        let ix = TestnetIx::RemoveAdmin {
            target: Pubkey::default(),
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 8);
    }

    #[test]
    fn test_add_keeper_discriminant() {
        let ix = TestnetIx::AddKeeper {
            target: Pubkey::default(),
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 9); // AddKeeper is the 10th variant (0-indexed)
    }

    #[test]
    fn test_remove_keeper_discriminant() {
        let ix = TestnetIx::RemoveKeeper {
            target: Pubkey::default(),
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 10);
    }

    #[test]
    fn test_update_orderbook_snapshot_discriminant() {
        let ix = TestnetIx::UpdateOrderBookSnapshot {
            market_id: 1,
            n_sig_figs: 4,
            bids: vec![],
            asks: vec![],
        };
        let data = ix.try_to_vec().unwrap();
        assert_eq!(data[0], 37);
    }
}
