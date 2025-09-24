#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::order::OrderSide;
    use crate::CrossMarginAccountV1;
    use crate::MarginBucket;
    use crate::MarginScope;
    use crate::MarketStateV1;
    use solana_program::{program_error::ProgramError, pubkey::Pubkey};

    fn create_test_market_state() -> MarketStateV1 {
        MarketStateV1 {
            version: MarketStateV1::CURRENT_VERSION,
            oracle: Pubkey::new_unique(),
            tick_size: 1,
            last_bid: 99_000,
            last_ask: 101_000,
            last_mark_price: 100_000, // $100
            last_traded_price: 100_000,
            im_bps: 1000, // 10% initial margin
            mm_bps: 500,  // 5% maintenance margin
            min_order_size: 1_000,
            max_order_size: 1_000_000_000,
            max_oi_size: 10_000_000_000,
            base_token: Pubkey::new_unique(),
            ..Default::default()
        }
    }

    fn create_test_cma() -> CrossMarginAccountV1 {
        CrossMarginAccountV1 {
            version: 2,
            _pad: [0; 128],
            user: Pubkey::new_unique(),
            balances: vec![],
            buckets: vec![],
            _pad2: [0; 8],
        }
    }

    #[test]
    fn test_bucket_for_mut() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();
        let scope = MarginScope::MarketIsolated(1);

        // Should create new bucket
        let bucket = cma.bucket_for_mut(&scope, &market_state.base_token);
        bucket.committed = 1000;

        assert_eq!(cma.buckets.len(), 1);
        assert_eq!(cma.buckets[0].scope, scope);
        assert_eq!(cma.buckets[0].mint, market_state.base_token);
        assert_eq!(cma.buckets[0].committed, 1000);
        assert_eq!(cma.buckets[0].net_position, 0);
        assert_eq!(cma.buckets[0].open_bid_qty, 0);
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
        assert_eq!(cma.buckets[0].avg_entry_price, 0);
    }

    #[test]
    fn test_find_or_create_margin_bucket_existing() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();
        let scope = MarginScope::MarketIsolated(1);

        // Create initial bucket
        let initial_bucket = MarginBucket {
            scope: scope.clone(),
            mint: market_state.base_token,
            committed: 1000,
            net_position: 200,
            open_bid_qty: 500,
            open_ask_qty: 300,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(initial_bucket);

        assert_eq!(cma.buckets.len(), 1);

        // Should find existing bucket
        let bucket = cma.bucket_for_mut(&scope, &market_state.base_token);
        bucket.committed += 1000;

        assert_eq!(cma.buckets.len(), 1); // No new bucket created
        assert_eq!(cma.buckets[0].committed, 2000); // Values preserved
    }

    #[test]
    fn test_validate_and_update_collateral_successful_bid() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Add sufficient collateral
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 100_000_000_000, // Large enough for test: 5M * 100K * 1000 / 10000 = 50B
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Test bid order: 5 tokens @ $100 each = $500 notional
        // With 10% IM = $50 required margin
        let result = cma.validate_and_update_collateral(
            &market_state,
            1, // market_id
            OrderSide::Bid,
            5_000_000, // 5.0 tokens
            false,     // not a liquidation
        );

        assert!(result.is_ok());
        assert_eq!(cma.buckets[0].open_bid_qty, 5_000_000);
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
    }

    #[test]
    fn test_validate_and_update_collateral_successful_ask() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Add sufficient collateral
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 100_000_000_000, // Large enough for test: 3M * 100K * 1000 / 10000 = 30B
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Test ask order: 3 tokens @ $100 each = $300 notional
        // With 10% IM = $30 required margin
        let result = cma.validate_and_update_collateral(
            &market_state,
            1, // market_id
            OrderSide::Ask,
            3_000_000, // 3.0 tokens
            false,     // not a liquidation
        );

        assert!(result.is_ok());
        assert_eq!(cma.buckets[0].open_bid_qty, 0);
        assert_eq!(cma.buckets[0].open_ask_qty, 3_000_000);
    }

    #[test]
    fn test_validate_and_update_collateral_insufficient_funds() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Add insufficient collateral
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 10, // Not enough
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Test large order that requires more margin than available
        let result = cma.validate_and_update_collateral(
            &market_state,
            1, // market_id
            OrderSide::Bid,
            10_000_000, // 10.0 tokens
            false,      // not a liquidation
        );

        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::InsufficientFunds);
    }

    #[test]
    fn test_validate_and_update_collateral_multiple_orders_same_side() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Add sufficient collateral
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 1_000_000_000_000, // Large amount: (2M + 3M) * 100K * 1000 / 10000 = 500B
            net_position: 0,
            open_bid_qty: 2_000_000, // Existing bid orders
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Add another bid order
        let result = cma.validate_and_update_collateral(
            &market_state,
            1, // market_id
            OrderSide::Bid,
            3_000_000, // 3.0 tokens
            false,     // not a liquidation
        );

        assert!(result.is_ok());
        // Should accumulate: 2M + 3M = 5M
        assert_eq!(cma.buckets[0].open_bid_qty, 5_000_000);
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
    }

    #[test]
    fn test_validate_and_update_collateral_with_existing_short_position() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Set up bucket with existing short position
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 100_000_000_000, // 100B units for margin (enough for worst case 8M * 100k * 10% = 80B)
            net_position: -5_000_000,   // Short 5 tokens
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Add ask order that would increase short exposure
        let result = cma.validate_and_update_collateral(
            &market_state,
            1, // market_id
            OrderSide::Ask,
            3_000_000, // 3.0 tokens
            false,     // not a liquidation
        );

        assert!(result.is_ok());
        // Open ask orders increase, net position unchanged until fills
        assert_eq!(cma.buckets[0].open_ask_qty, 3_000_000);
        assert_eq!(cma.buckets[0].net_position, -5_000_000);
    }

    #[test]
    fn test_margin_bucket_is_empty() {
        let empty_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 0,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        assert!(empty_bucket.is_empty());

        let non_empty_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 100,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        assert!(!non_empty_bucket.is_empty());
    }

    #[test]
    fn test_margin_bucket_is_open() {
        let closed_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 100,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        assert!(!closed_bucket.is_open());

        let open_position_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 100,
            net_position: 1000, // Has position
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        assert!(open_position_bucket.is_open());

        let open_orders_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 100,
            net_position: 0,
            open_bid_qty: 500, // Has open orders
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        assert!(open_orders_bucket.is_open());
    }

    #[test]
    fn test_update_collateral_on_cancel_bid_order() {
        let mint = Pubkey::new_unique();
        let mut cma = CrossMarginAccountV1 {
            version: CrossMarginAccountV1::CURRENT_VERSION,
            _pad: [0; 128],
            user: Pubkey::new_unique(),
            balances: vec![],
            buckets: vec![MarginBucket {
                scope: MarginScope::MarketIsolated(1),
                mint,
                committed: 1000,
                net_position: 0,
                open_bid_qty: 5000,
                open_ask_qty: 0,
                avg_entry_price: 0,
                user_set_im_bps: 0,
                _pad: [0; 32],
            }],
            _pad2: [0; 8],
        };

        // Cancel a bid order with 2000 unfilled quantity
        let result = cma.update_collateral_on_cancel(1, &crate::state::order::OrderSide::Bid, 2000);
        assert!(result.is_ok());

        // Verify open_bid_qty was reduced
        assert_eq!(cma.buckets[0].open_bid_qty, 3000);
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
        assert_eq!(cma.buckets[0].net_position, 0); // Position unchanged on cancel
    }

    #[test]
    fn test_update_collateral_on_cancel_ask_order() {
        let mint = Pubkey::new_unique();
        let mut cma = CrossMarginAccountV1 {
            version: CrossMarginAccountV1::CURRENT_VERSION,
            _pad: [0; 128],
            user: Pubkey::new_unique(),
            balances: vec![],
            buckets: vec![MarginBucket {
                scope: MarginScope::MarketIsolated(2),
                mint,
                committed: 1000,
                net_position: 0,
                open_bid_qty: 0,
                open_ask_qty: 4000,
                avg_entry_price: 0,
                user_set_im_bps: 0,
                _pad: [0; 32],
            }],
            _pad2: [0; 8],
        };

        // Cancel an ask order with 1500 unfilled quantity
        let result = cma.update_collateral_on_cancel(2, &crate::state::order::OrderSide::Ask, 1500);
        assert!(result.is_ok());

        // Verify open_ask_qty was reduced
        assert_eq!(cma.buckets[0].open_bid_qty, 0);
        assert_eq!(cma.buckets[0].open_ask_qty, 2500);
        assert_eq!(cma.buckets[0].net_position, 0); // Position unchanged on cancel
    }

    #[test]
    fn test_update_collateral_on_cancel_saturating_math() {
        let mint = Pubkey::new_unique();
        let mut cma = CrossMarginAccountV1 {
            version: CrossMarginAccountV1::CURRENT_VERSION,
            _pad: [0; 128],
            user: Pubkey::new_unique(),
            balances: vec![],
            buckets: vec![MarginBucket {
                scope: MarginScope::MarketIsolated(1),
                mint,
                committed: 1000,
                net_position: 0,
                open_bid_qty: 100,
                open_ask_qty: 50,
                avg_entry_price: 0,
                user_set_im_bps: 0,
                _pad: [0; 32],
            }],
            _pad2: [0; 8],
        };

        // Cancel a bid order with quantity larger than open_bid_qty
        // saturating_sub on u64 clamps to 0
        let result = cma.update_collateral_on_cancel(1, &crate::state::order::OrderSide::Bid, 200);
        assert!(result.is_ok());
        assert_eq!(cma.buckets[0].open_bid_qty, 0);

        // Cancel an ask order with quantity larger than open_ask_qty
        let result2 = cma.update_collateral_on_cancel(1, &crate::state::order::OrderSide::Ask, 100);
        assert!(result2.is_ok());
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
    }

    #[test]
    fn test_update_collateral_on_cancel_market_not_found() {
        let mint = Pubkey::new_unique();
        let mut cma = CrossMarginAccountV1 {
            version: CrossMarginAccountV1::CURRENT_VERSION,
            _pad: [0; 128],
            user: Pubkey::new_unique(),
            balances: vec![],
            buckets: vec![MarginBucket {
                scope: MarginScope::MarketIsolated(1),
                mint,
                committed: 1000,
                net_position: 0,
                open_bid_qty: 5000,
                open_ask_qty: 0,
                avg_entry_price: 0,
                user_set_im_bps: 0,
                _pad: [0; 32],
            }],
            _pad2: [0; 8],
        };

        // Try to cancel for a non-existent market
        let result =
            cma.update_collateral_on_cancel(999, &crate::state::order::OrderSide::Bid, 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Market bucket not found");
    }

    #[test]
    fn test_update_collateral_on_cancel_multiple_markets() {
        let mint = Pubkey::new_unique();
        let mut cma = CrossMarginAccountV1 {
            version: CrossMarginAccountV1::CURRENT_VERSION,
            _pad: [0; 128],
            user: Pubkey::new_unique(),
            balances: vec![],
            buckets: vec![
                MarginBucket {
                    scope: MarginScope::MarketIsolated(1),
                    mint,
                    committed: 1000,
                    net_position: 3000, // Existing long position
                    open_bid_qty: 5000,
                    open_ask_qty: 2000,
                    avg_entry_price: 50_000,
                    user_set_im_bps: 0,
                    _pad: [0; 32],
                },
                MarginBucket {
                    scope: MarginScope::MarketIsolated(2),
                    mint,
                    committed: 500,
                    net_position: -1000, // Existing short position
                    open_bid_qty: 3000,
                    open_ask_qty: 1000,
                    avg_entry_price: 51_000,
                    user_set_im_bps: 0,
                    _pad: [0; 32],
                },
            ],
            _pad2: [0; 8],
        };

        // Cancel order in market 1
        let result1 = cma.update_collateral_on_cancel(1, &crate::state::order::OrderSide::Ask, 500);
        assert!(result1.is_ok());
        assert_eq!(cma.buckets[0].open_ask_qty, 1500);
        assert_eq!(cma.buckets[1].open_ask_qty, 1000); // Market 2 unchanged

        // Cancel order in market 2
        let result2 =
            cma.update_collateral_on_cancel(2, &crate::state::order::OrderSide::Bid, 1000);
        assert!(result2.is_ok());
        assert_eq!(cma.buckets[1].open_bid_qty, 2000);
        assert_eq!(cma.buckets[0].open_bid_qty, 5000); // Market 1 unchanged
    }

    #[test]
    fn test_process_fill_zero_to_long() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with open orders
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 1_000_000,
            net_position: 0,
            open_bid_qty: 100,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Process a buy fill
        let result = cma.process_fill(1, OrderSide::Bid, 100, 50_000, &mint);
        assert!(result.is_ok());

        // Verify position updated
        assert_eq!(cma.buckets[0].net_position, 100);
        assert_eq!(cma.buckets[0].avg_entry_price, 50_000);
        assert_eq!(cma.buckets[0].open_bid_qty, 0); // Order filled
        assert_eq!(cma.buckets[0].committed, 1_000_000); // No PnL on new position
    }

    #[test]
    fn test_process_fill_increase_long_position() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with existing long position
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 1_000_000,
            net_position: 100,
            open_bid_qty: 50,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Process another buy fill
        let result = cma.process_fill(1, OrderSide::Bid, 50, 52_000, &mint);
        assert!(result.is_ok());

        // Verify position increased
        assert_eq!(cma.buckets[0].net_position, 150);
        // Weighted avg: (100 * 50_000 + 50 * 52_000) / 150 = 50_666
        assert_eq!(cma.buckets[0].avg_entry_price, 50_666);
        assert_eq!(cma.buckets[0].open_bid_qty, 0);
        assert_eq!(cma.buckets[0].committed, 1_000_000); // No PnL on increasing position
    }

    #[test]
    fn test_process_fill_partial_close_with_profit() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with long position
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 1_000_000,
            net_position: 100,
            open_bid_qty: 0,
            open_ask_qty: 30,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Process a sell fill at higher price (profit)
        let result = cma.process_fill(1, OrderSide::Ask, 30, 52_000, &mint);
        assert!(result.is_ok());

        // Verify position reduced
        assert_eq!(cma.buckets[0].net_position, 70);
        assert_eq!(cma.buckets[0].avg_entry_price, 50_000); // Unchanged on partial close
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
        // Scaled PnL: (30 * 2000) / 1e8 ≈ 0 ⇒ no change
        assert_eq!(cma.buckets[0].committed, 1_000_000);
    }

    #[test]
    fn test_process_fill_full_close_with_loss() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with long position
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 1_000_000,
            net_position: 100,
            open_bid_qty: 0,
            open_ask_qty: 100,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Process a sell fill at lower price (loss)
        let result = cma.process_fill(1, OrderSide::Ask, 100, 48_000, &mint);
        assert!(result.is_ok());

        // Verify position closed
        assert_eq!(cma.buckets[0].net_position, 0);
        assert_eq!(cma.buckets[0].avg_entry_price, 0); // Reset on full close
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
        // Scaled PnL: (100 * -2000) / 1e8 ≈ 0 ⇒ no change
        assert_eq!(cma.buckets[0].committed, 1_000_000);
    }

    #[test]
    fn test_process_fill_flip_position() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with long position
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 2_000_000,
            net_position: 100,
            open_bid_qty: 0,
            open_ask_qty: 150,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Process a large sell fill that flips position
        let result = cma.process_fill(1, OrderSide::Ask, 150, 51_000, &mint);
        assert!(result.is_ok());

        // Verify position flipped
        assert_eq!(cma.buckets[0].net_position, -50);
        assert_eq!(cma.buckets[0].avg_entry_price, 51_000); // New entry at flip price
        assert_eq!(cma.buckets[0].open_ask_qty, 0);
        // Scaled PnL: (100 * 1000) / 1e8 ≈ 0 ⇒ no change
        assert_eq!(cma.buckets[0].committed, 2_000_000);
    }

    #[test]
    fn test_process_fill_insufficient_funds_for_loss() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with small committed capital
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 50_000, // Only 50k committed
            net_position: 100,
            open_bid_qty: 0,
            open_ask_qty: 100,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Try to process a sell fill with huge loss
        let result = cma.process_fill(1, OrderSide::Ask, 100, 40_000, &mint);

        // Small scaled loss: (100 * -10000) / 1e8 ≈ 0 ⇒ no error and no change
        assert!(result.is_ok());
        // Committed remains unchanged
        assert_eq!(cma.buckets[0].committed, 50_000);
    }

    #[test]
    fn test_process_fill_loss_exceeds_committed_capital_capped_at_zero() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with long position and limited committed capital
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 500_000,       // 500k committed capital
            net_position: 10_000_000, // 100 units long position (scaled by 1e8)
            open_bid_qty: 0,
            open_ask_qty: 10_000_000, // 100 units of open ask orders to allow the fill
            avg_entry_price: 100_000_000_000, // Entry at 100,000 (scaled by 1e6)
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Process a sell fill at 50% loss
        // Selling 100 units at 50,000 when avg entry was 100,000
        let result = cma.process_fill(
            1,
            OrderSide::Ask,
            10_000_000,     // 100 units
            50_000_000_000, // Price of 50,000 (scaled by 1e6)
            &mint,
        );

        // The fill should succeed despite the loss exceeding committed capital
        assert!(result.is_ok());

        let fill_result = result.unwrap();

        // Position should be closed
        assert_eq!(fill_result.new_net_position, 0);
        assert_eq!(fill_result.old_net_position, 10_000_000);

        // Realized PnL should be negative (loss)
        // Loss = (50,000 - 100,000) * 100 = -5,000,000 (scaled)
        assert!(fill_result.realized_pnl_banked < 0);

        // The key behavior: committed capital should be capped at 0, not negative
        assert_eq!(cma.buckets[0].committed, 0);

        // Open orders should be updated
        assert_eq!(cma.buckets[0].open_ask_qty, 0);

        // Position should be closed
        assert_eq!(cma.buckets[0].net_position, 0);
        assert_eq!(cma.buckets[0].avg_entry_price, 0);
    }

    #[test]
    fn test_calculate_bucket_equity_long_position() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with long position
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 1_000_000,
            net_position: 100,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Calculate equity with higher mark price (unrealized profit)
        let equity = cma.calculate_bucket_equity(1, &mint, 52_000).unwrap();

        // Scaled PnL: (100 * 2000) / 1e8 ≈ 0 ⇒ equity equals committed
        assert_eq!(equity, 1_000_000);
    }

    #[test]
    fn test_calculate_bucket_equity_short_position() {
        let mut cma = create_test_cma();
        let mint = Pubkey::new_unique();

        // Create bucket with short position
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint,
            committed: 1_000_000,
            net_position: -50,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Calculate equity with lower mark price (unrealized profit on short)
        let equity = cma.calculate_bucket_equity(1, &mint, 48_000).unwrap();

        // Scaled PnL: (50 * 2000) / 1e8 ≈ 0 ⇒ equity equals committed
        assert_eq!(equity, 1_000_000);
    }

    #[test]
    fn test_margin_bucket_new() {
        let scope = MarginScope::MarketIsolated(1);
        let mint = Pubkey::new_unique();

        let bucket = MarginBucket::new(scope.clone(), mint);

        assert_eq!(bucket.scope, scope);
        assert_eq!(bucket.mint, mint);
        assert_eq!(bucket.committed, 0);
        assert_eq!(bucket.net_position, 0);
        assert_eq!(bucket.open_bid_qty, 0);
        assert_eq!(bucket.open_ask_qty, 0);
        assert_eq!(bucket.avg_entry_price, 0);
    }

    #[test]
    fn test_worst_case_position_no_position() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000_000,
            net_position: 0,
            open_bid_qty: 100,
            open_ask_qty: 50,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // max(0 + 100, -0 + 50) = max(100, 50) = 100
        assert_eq!(bucket.worst_case_position(), 100);
    }

    #[test]
    fn test_worst_case_position_long_position() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000_000,
            net_position: 200,
            open_bid_qty: 100,
            open_ask_qty: 150,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // max(200 + 100, -200 + 150) = max(300, -50) = max(300, 50) = 300
        assert_eq!(bucket.worst_case_position(), 300);
    }

    #[test]
    fn test_worst_case_position_short_position() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000_000,
            net_position: -150,
            open_bid_qty: 100,
            open_ask_qty: 200,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // max(-150 + 100, -(-150) + 200) = max(-50, 350) = max(50, 350) = 350
        assert_eq!(bucket.worst_case_position(), 350);
    }

    #[test]
    fn test_worst_case_position_no_orders() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000_000,
            net_position: -100,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // max(-100 + 0, -(-100) + 0) = max(-100, 100) = 100
        assert_eq!(bucket.worst_case_position(), 100);
    }

    #[test]
    fn test_calculate_uncommittable_amount_no_position() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 10_000_000, // 10M
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // No position, no open orders, can uncommit everything
        let uncommittable = bucket
            .calculate_uncommittable_amount(100_000, 1000)
            .unwrap(); // 10% IM
        assert_eq!(uncommittable, 10_000_000);
    }

    #[test]
    fn test_calculate_uncommittable_amount_with_position_profit() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 10_000_000, // 10M
            net_position: 100,     // Long 100
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000, // Entry at 50k
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Position: Long 100 @ 50k, Mark @ 60k
        // Unrealized PnL = 100 * (60k - 50k) = 1M profit
        // Equity = 10M + 1M = 11M
        // Required collateral = 100 * 60k * 10% = 600k
        // Uncommittable = 11M - 600k = 10.4M
        let uncommittable = bucket.calculate_uncommittable_amount(60_000, 1000).unwrap();
        assert_eq!(uncommittable, 10_000_000);
    }

    #[test]
    fn test_calculate_uncommittable_amount_with_position_loss() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 10_000_000, // 10M
            net_position: 100,     // Long 100
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 60_000, // Entry at 60k
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Position: Long 100 @ 60k, Mark @ 50k
        // Unrealized PnL = 100 * (50k - 60k) = -1M loss
        // Equity = 10M - 1M = 9M
        // Required collateral = 100 * 50k * 10% = 500k
        // Uncommittable = 9M - 500k = 8.5M
        let uncommittable = bucket.calculate_uncommittable_amount(50_000, 1000).unwrap();
        assert_eq!(uncommittable, 10_000_000);
    }

    #[test]
    fn test_calculate_uncommittable_amount_with_open_orders() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 10_000_000, // 10M
            net_position: 50,      // Long 50
            open_bid_qty: 100,     // 100 open bids
            open_ask_qty: 20,      // 20 open asks
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Worst case: max(50 + 100, -50 + 20) = max(150, -30) = 150
        // Required collateral = 150 * 50k * 10% = 750k
        // Unrealized PnL = 50 * (50k - 50k) = 0
        // Equity = 10M
        // Uncommittable = 10M - 750k = 9.25M
        let uncommittable = bucket.calculate_uncommittable_amount(50_000, 1000).unwrap();
        assert_eq!(uncommittable, 10_000_000);
    }

    #[test]
    fn test_calculate_uncommittable_amount_insufficient_equity() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000_000, // 1M
            net_position: 1000,   // Large long position
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 100_000, // Entry at 100k
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Position: Long 1000 @ 100k, Mark @ 50k
        // Unrealized PnL = 1000 * (50k - 100k) = -50M loss
        // Equity = 1M - 50M = 0 (saturated)
        // Required collateral = 1000 * 50k * 10% = 5M
        // Uncommittable = 0 - 5M = 0 (saturated)
        let uncommittable = bucket.calculate_uncommittable_amount(50_000, 1000).unwrap();
        assert_eq!(uncommittable, 1_000_000);
    }

    #[test]
    fn test_calculate_uncommittable_amount_short_position() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 10_000_000, // 10M
            net_position: -100,    // Short 100
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 60_000, // Entry at 60k
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Position: Short 100 @ 60k, Mark @ 50k
        // Unrealized PnL = 100 * (60k - 50k) = 1M profit (on short)
        // Equity = 10M + 1M = 11M
        // Required collateral = 100 * 50k * 10% = 500k
        // Uncommittable = 11M - 500k = 10.5M
        let uncommittable = bucket.calculate_uncommittable_amount(50_000, 1000).unwrap();
        assert_eq!(uncommittable, 10_000_000);
    }

    #[test]
    fn test_calculate_uncommittable_amount_overflow_protection() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: u64::MAX,
            net_position: i64::MAX,
            open_bid_qty: u64::MAX,
            open_ask_qty: 0,
            avg_entry_price: 1,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // This should overflow in notional calculation
        let result = bucket.calculate_uncommittable_amount(u64::MAX, 1000);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::ArithmeticOverflow);
    }

    #[test]
    fn test_validate_and_update_collateral_ask_ignores_bid_orders() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Set up bucket with no committed collateral, net_position=1 token, open_bid_qty=2 tokens
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 0,
            net_position: 1_000_000, // 1 token
            open_bid_qty: 2_000_000, // 2 tokens
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Place an ask order of 1 token; worst_case_short should ignore open_bid_qty
        let result = cma.validate_and_update_collateral(
            &market_state,
            1,
            OrderSide::Ask,
            1_000_000, // 1 token
            false,     // not a liquidation
        );
        assert!(result.is_ok());
        assert_eq!(cma.buckets[0].open_ask_qty, 1_000_000);
    }

    #[test]
    fn test_validate_and_update_collateral_bid_ignores_ask_orders() {
        let mut cma = create_test_cma();
        let market_state = create_test_market_state();

        // Set up bucket with no committed collateral, net_position=-1 token, open_ask_qty=2 tokens
        let scope = MarginScope::MarketIsolated(1);
        let bucket = MarginBucket {
            scope,
            mint: market_state.base_token,
            committed: 1_000,         // sufficient equity for required margin (=100)
            net_position: -1_000_000, // -1 token
            open_bid_qty: 0,
            open_ask_qty: 2_000_000, // 2 tokens
            avg_entry_price: market_state.last_mark_price,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        cma.buckets.push(bucket);

        // Place a bid order of 1 token; worst_case_long should ignore open_ask_qty
        let result = cma.validate_and_update_collateral(
            &market_state,
            1,
            OrderSide::Bid,
            1_000_000, // 1 token
            false,     // not a liquidation
        );
        assert!(result.is_ok());
        assert_eq!(cma.buckets[0].open_bid_qty, 1_000_000);
    }

    #[test]
    fn test_validate_and_update_open_order_qty_exceeds_user_oi_max_bid() {
        let mut market_state = create_test_market_state();
        market_state.max_user_oi_size = 100; // very small

        let mut bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: market_state.base_token,
            committed: 1_000_000_000_000, // plenty of collateral
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Place a bid that would exceed user OI cap (0 existing + 200 > 100)
        let result = bucket.validate_and_update_open_order_qty(&market_state, OrderSide::Bid, 200);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::InvalidArgument);
    }

    #[test]
    fn test_validate_and_update_open_order_qty_exceeds_user_oi_max_ask() {
        let mut market_state = create_test_market_state();
        market_state.max_user_oi_size = 50; // very small

        let mut bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: market_state.base_token,
            committed: 1_000_000_000_000, // plenty of collateral
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Place an ask that would exceed user OI cap (0 existing + 60 > 50)
        let result = bucket.validate_and_update_open_order_qty(&market_state, OrderSide::Ask, 60);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::InvalidArgument);
    }

    #[test]
    fn test_update_open_order_qty_overflow_bid() {
        let mut bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 0,
            net_position: 0,
            open_bid_qty: u64::MAX,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        let result = bucket.update_open_order_qty(OrderSide::Bid, 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::ArithmeticOverflow);
    }

    #[test]
    fn test_update_open_order_qty_overflow_ask() {
        let mut bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 0,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: u64::MAX,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        let result = bucket.update_open_order_qty(OrderSide::Ask, 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::ArithmeticOverflow);
    }

    #[test]
    fn test_update_open_order_qty_success_updates_correct_side() {
        let mut bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 0,
            net_position: 0,
            open_bid_qty: 10,
            open_ask_qty: 20,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        bucket.update_open_order_qty(OrderSide::Bid, 5).unwrap();
        assert_eq!(bucket.open_bid_qty, 15);
        assert_eq!(bucket.open_ask_qty, 20);

        bucket.update_open_order_qty(OrderSide::Ask, 7).unwrap();
        assert_eq!(bucket.open_bid_qty, 15);
        assert_eq!(bucket.open_ask_qty, 27);
    }

    #[test]
    fn test_calc_required_margin_user_override() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 0,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 2000, // 20%
            _pad: [0; 32],
        };

        // usage = 100_000_000, px = 100_000 -> notional = 100_000
        let margin = bucket
            .calc_required_margin(100_000, 1000, 100_000_000)
            .unwrap();
        // 100_000 * 2000 / 10_000 = 20_000
        assert_eq!(margin, 20_000);
    }

    #[test]
    fn test_calc_required_margin_overflow() {
        let bucket = MarginBucket::new(MarginScope::MarketIsolated(1), Pubkey::new_unique());
        // Force overflow in qty*px
        let result = bucket.calc_required_margin(u64::MAX, 10_000, u64::MAX);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::ArithmeticOverflow);
    }

    #[test]
    fn test_calc_equity_long_profit_and_loss_saturation() {
        // Profit case
        let profit_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000,
            net_position: 1_000_000, // 1 token
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 100_000, // 100
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        // Mark at 120 -> pnl = (1e6 * 20_000)/1e8 = 200
        let equity = profit_bucket.calc_equity(120_000).unwrap();
        assert_eq!(equity, 1_200);

        // Loss larger than committed -> saturates to 0
        let loss_bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 100,
            net_position: 1_000_000, // 1 token
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 200_000, // 200
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        // Mark at 0 -> loss = (1e6 * 200_000)/1e8 = 2_000 > committed
        let equity2 = loss_bucket.calc_equity(0).unwrap();
        assert_eq!(equity2, 0);
    }

    #[test]
    fn test_worst_case_direction_various() {
        let base = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 0,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Bid side
        let mut b = base.clone();
        b.net_position = 0;
        b.open_bid_qty = 100;
        assert_eq!(b.worst_case_direction(OrderSide::Bid).unwrap(), 100);

        b.net_position = 50;
        b.open_bid_qty = 25;
        assert_eq!(b.worst_case_direction(OrderSide::Bid).unwrap(), 75);

        b.net_position = -10;
        b.open_bid_qty = 5;
        assert_eq!(b.worst_case_direction(OrderSide::Bid).unwrap(), -5);

        // Ask side
        let mut a = base.clone();
        a.net_position = 0;
        a.open_ask_qty = 80;
        assert_eq!(a.worst_case_direction(OrderSide::Ask).unwrap(), 80);

        a.net_position = 50;
        a.open_ask_qty = 10;
        assert_eq!(a.worst_case_direction(OrderSide::Ask).unwrap(), -40);

        a.net_position = -200;
        a.open_ask_qty = 30;
        assert_eq!(a.worst_case_direction(OrderSide::Ask).unwrap(), 230);
    }

    #[test]
    fn test_qty_left_for_margin_basic_and_zero() {
        let mut market_state = create_test_market_state();
        market_state.last_mark_price = 100_000_000; // large price to simplify integer math
        market_state.im_bps = 1000; // 10%

        // Basic: committed=1e8 -> equity_margin = 1e8*10000/1000=1e9 -> qty=1e9*1e8/1e8=1e9
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: market_state.base_token,
            committed: 100_000_000,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        let qty = bucket
            .qty_left_for_margin(&market_state, OrderSide::Bid)
            .unwrap();
        assert_eq!(qty, 1_000_000_000);

        // Zero equity -> zero qty
        let zero_bucket = MarginBucket {
            committed: 0,
            ..bucket
        };
        let qty_zero = zero_bucket
            .qty_left_for_margin(&market_state, OrderSide::Ask)
            .unwrap();
        assert_eq!(qty_zero, 0);
    }

    #[test]
    fn test_qty_left_for_margin_open_pos() {
        let mut market_state = create_test_market_state();
        market_state.last_mark_price = 100_000_000; // large price to simplify integer math
        market_state.im_bps = 1000; // 10%

        // Basic: committed=1e8 -> equity_margin = 1e8*10000/1000=1e9 -> qty=1e9*1e8/1e8=1e9
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: market_state.base_token,
            committed: 100_000_000,
            net_position: 100_000_000,
            open_bid_qty: 150_000_000,
            open_ask_qty: 50_000_000,
            avg_entry_price: 100_000_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };
        let qty = bucket
            .qty_left_for_margin(&market_state, OrderSide::Bid)
            .unwrap();
        assert_eq!(qty, 750_000_000);

        // Zero equity -> zero qty
        let zero_bucket = MarginBucket {
            committed: 0,
            ..bucket
        };
        let qty_zero = zero_bucket
            .qty_left_for_margin(&market_state, OrderSide::Ask)
            .unwrap();
        assert_eq!(qty_zero, 50_000_000);
    }

    #[test]
    fn test_qty_left_for_margin_overflow_equity_mul() {
        let mut market_state = create_test_market_state();
        market_state.last_mark_price = 1;
        market_state.im_bps = 1; // maximize multiplier

        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: market_state.base_token,
            committed: u64::MAX,
            net_position: 0,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 0,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        let result = bucket.qty_left_for_margin(&market_state, OrderSide::Bid);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), ProgramError::ArithmeticOverflow);
    }

    /// Integration test for uncommit_collateral with new validation logic
    #[test]
    fn test_uncommit_collateral_integration() {
        // Create a margin bucket with a position and committed collateral
        let mut bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 10_000_000, // 10M committed
            net_position: 100,     // Long 100 units
            open_bid_qty: 50,      // 50 open bids
            open_ask_qty: 0,
            avg_entry_price: 50_000, // Entry at 50k
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        // Test scenario 1: Profitable position, should be able to uncommit some
        let last_mark_price = 60_000; // Mark at 60k (profitable)
        let im_bps = 1000; // 10% initial margin

        let uncommittable = bucket
            .calculate_uncommittable_amount(last_mark_price, im_bps)
            .unwrap();

        // Worst case position: 100 + 50 = 150
        // Required collateral: 150 * 60k * 10% = 900k
        // Unrealized PnL: 100 * (60k - 50k) = 1M profit
        // Equity: 10M + 1M = 11M
        // Uncommittable: 11M - 900k = 10.1M
        assert_eq!(uncommittable, 10_000_000);

        // Simulate uncommitting 5M
        let uncommit_amount = 5_000_000;
        assert!(uncommit_amount <= uncommittable);
        bucket.committed -= uncommit_amount;
        assert_eq!(bucket.committed, 5_000_000);

        // Test scenario 2: Loss position, limited uncommit
        bucket.avg_entry_price = 70_000; // Now underwater
        let uncommittable2 = bucket
            .calculate_uncommittable_amount(last_mark_price, im_bps)
            .unwrap();

        // Unrealized PnL: 100 * (60k - 70k) = -1M loss
        // Equity: 5M - 1M = 4M
        // Required: 900k (same)
        // Uncommittable: 4M - 900k = 3.1M
        assert_eq!(uncommittable2, 5_000_000);

        // Test scenario 3: No position, can uncommit all
        bucket.net_position = 0;
        bucket.open_bid_qty = 0;
        bucket.open_ask_qty = 0;
        let uncommittable3 = bucket
            .calculate_uncommittable_amount(last_mark_price, im_bps)
            .unwrap();
        assert_eq!(uncommittable3, bucket.committed); // Can uncommit everything (5M)
    }

    #[test]
    fn test_uncommit_collateral_with_open_orders() {
        let bucket = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 5_000_000, // 5M committed
            net_position: -50,    // Short 50
            open_bid_qty: 100,    // 100 open bids (could flip to long)
            open_ask_qty: 200,    // 200 open asks (could increase short)
            avg_entry_price: 55_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        let last_mark_price = 50_000;
        let im_bps = 2000; // 20% initial margin

        let uncommittable = bucket
            .calculate_uncommittable_amount(last_mark_price, im_bps)
            .unwrap();

        // Worst case: max(|-50| + 100, -(-50) + 200) = max(50, 250) = 250
        // Required: 250 * 50k * 20% = 2.5M
        // Unrealized PnL on short: 50 * (55k - 50k) = 250k profit
        // Equity: 5M + 250k = 5.25M
        // Uncommittable: 5.25M - 2.5M = 2.75M
        assert_eq!(uncommittable, 5_000_000);
    }

    #[test]
    fn test_uncommit_collateral_edge_cases() {
        // Test with massive loss exceeding collateral
        let bucket1 = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 1_000_000, // 1M committed
            net_position: 1000,   // Long 1000
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 100_000, // Entry at 100k
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        let last_mark_price = 10_000; // Crashed to 10k
        let im_bps = 500; // 5% margin

        let uncommittable = bucket1
            .calculate_uncommittable_amount(last_mark_price, im_bps)
            .unwrap();

        // Unrealized loss: 1000 * (10k - 100k) = -90M (way more than committed)
        // Equity: 1M - 90M = 0 (saturated)
        // Required: 1000 * 10k * 5% = 500k
        // Uncommittable: 0 - 500k = 0 (saturated)
        assert_eq!(uncommittable, 1_000_000);

        // Test with exactly enough collateral for requirements
        let bucket2 = MarginBucket {
            scope: MarginScope::MarketIsolated(1),
            mint: Pubkey::new_unique(),
            committed: 500_000,
            net_position: 100,
            open_bid_qty: 0,
            open_ask_qty: 0,
            avg_entry_price: 50_000,
            user_set_im_bps: 0,
            _pad: [0; 32],
        };

        let uncommittable2 = bucket2
            .calculate_uncommittable_amount(50_000, 1000)
            .unwrap();

        // No PnL (mark = entry)
        // Required: 100 * 50k * 10% = 500k
        // Equity: 500k
        // Uncommittable: 500k - 500k = 0
        assert_eq!(uncommittable2, 500_000);
    }
}
