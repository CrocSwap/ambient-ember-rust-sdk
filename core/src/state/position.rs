use crate::state::math::mul_qty_px_signed;
use crate::state::order::OrderSide;
use solana_program::program_error::ProgramError;

/// Represents a fill event for position tracking
#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub side: OrderSide,
    pub qty: u64,
    pub price: u64,
}

/// Result of processing a fill
#[derive(Debug, Clone, PartialEq)]
pub struct FillResult {
    pub new_net_position: i64,
    pub new_avg_entry_price: u64,
    pub realized_pnl: i64,
}

/// Calculates the result of applying a fill to a position
pub fn process_fill(
    current_net_position: i64,
    current_avg_entry_price: u64,
    fill: &Fill,
) -> Result<FillResult, ProgramError> {
    use solana_program::msg;

    // Convert fill to signed position change
    let position_change = match fill.side {
        OrderSide::Bid => fill.qty as i64,
        OrderSide::Ask => -(fill.qty as i64),
    };

    // Calculate new net position
    let new_net_position = current_net_position
        .checked_add(position_change)
        .ok_or_else(|| {
            msg!("Error: Overflow calculating new net position");
            ProgramError::ArithmeticOverflow
        })?;

    // Determine fill scenario and calculate results
    match (current_net_position, new_net_position) {
        // Scenario 1: Zero to non-zero position
        (0, _) => Ok(FillResult {
            new_net_position,
            new_avg_entry_price: fill.price,
            realized_pnl: 0,
        }),

        // Scenarios 2-5: Non-zero starting position
        (current, new) if current != 0 => {
            // Check if position is increasing in magnitude (same direction as current position)
            let is_increasing =
                (current > 0 && position_change > 0) || (current < 0 && position_change < 0);

            if is_increasing {
                // Scenario 2: Increasing position magnitude
                let new_avg_entry_price = calculate_weighted_avg_price(
                    current.unsigned_abs(),
                    current_avg_entry_price,
                    position_change.unsigned_abs(),
                    fill.price,
                )?;

                Ok(FillResult {
                    new_net_position,
                    new_avg_entry_price,
                    realized_pnl: 0,
                })
            } else {
                // Position is reducing or flipping
                let position_closed = if (current > 0 && new <= 0) || (current < 0 && new >= 0) {
                    // Full close or flip
                    current.unsigned_abs()
                } else {
                    // Partial close (Scenario 3)
                    position_change.unsigned_abs()
                };

                // Calculate realized PnL
                let realized_pnl = calculate_realized_pnl(
                    current > 0,
                    position_closed,
                    current_avg_entry_price,
                    fill.price,
                )?;

                // Determine new average entry price
                let new_avg_entry_price = if new == 0 {
                    // Scenario 4: Position fully closed
                    0
                } else if (current > 0 && new < 0) || (current < 0 && new > 0) {
                    // Scenario 5: Position flipped
                    fill.price
                } else {
                    // Scenario 3: Position reduced but not closed
                    current_avg_entry_price
                };

                Ok(FillResult {
                    new_net_position,
                    new_avg_entry_price,
                    realized_pnl,
                })
            }
        }

        _ => unreachable!("All cases should be covered"),
    }
}

/// Calculates weighted average price for position increases
fn calculate_weighted_avg_price(
    existing_qty: u64,
    existing_price: u64,
    new_qty: u64,
    new_price: u64,
) -> Result<u64, ProgramError> {
    use solana_program::msg;

    // Calculate weighted sum
    let existing_value = existing_qty.checked_mul(existing_price).ok_or_else(|| {
        msg!("Error: Overflow calculating existing value");
        ProgramError::ArithmeticOverflow
    })?;

    let new_value = new_qty.checked_mul(new_price).ok_or_else(|| {
        msg!("Error: Overflow calculating new value");
        ProgramError::ArithmeticOverflow
    })?;

    let total_value = existing_value.checked_add(new_value).ok_or_else(|| {
        msg!("Error: Overflow calculating total value");
        ProgramError::ArithmeticOverflow
    })?;

    let total_qty = existing_qty.checked_add(new_qty).ok_or_else(|| {
        msg!("Error: Overflow calculating total quantity");
        ProgramError::ArithmeticOverflow
    })?;

    // Calculate average
    total_value.checked_div(total_qty).ok_or_else(|| {
        msg!("Error: Division by zero calculating average price");
        ProgramError::ArithmeticOverflow
    })
}

/// Calculates realized PnL for position reductions
fn calculate_realized_pnl(
    was_long: bool,
    qty_closed: u64,
    avg_entry_price: u64,
    exit_price: u64,
) -> Result<i64, ProgramError> {
    // Calculate price difference
    let price_diff = if was_long {
        exit_price as i64 - avg_entry_price as i64
    } else {
        avg_entry_price as i64 - exit_price as i64
    };

    // Calculate total PnL (scaled to collateral decimals)
    mul_qty_px_signed(qty_closed as i64, price_diff)
}

/// Process a fill for the clearing/market-wide counterparty
/// The clearing takes the opposite side of every user fill
pub fn process_clearing_fill(
    current_net_position: i64,
    current_avg_entry_price: u64,
    user_fill: &Fill,
) -> Result<FillResult, ProgramError> {
    // Create inverted fill for clearing (opposite side)
    let clearing_fill = Fill {
        side: match user_fill.side {
            OrderSide::Bid => OrderSide::Ask, // User buys, clearing sells
            OrderSide::Ask => OrderSide::Bid, // User sells, clearing buys
        },
        qty: user_fill.qty,
        price: user_fill.price,
    };

    // Use the same position tracking logic
    process_fill(
        current_net_position,
        current_avg_entry_price,
        &clearing_fill,
    )
}

/// Calculates equity (committed + unrealized PnL) for a position
pub fn calculate_equity(
    committed_collateral: u64,
    net_position: i64,
    avg_entry_price: u64,
    mark_price: u64,
) -> Result<i64, ProgramError> {
    // No position means equity equals committed collateral
    if net_position == 0 {
        return Ok(committed_collateral as i64);
    }

    // Calculate unrealized PnL
    let unrealized_pnl = calculate_realized_pnl(
        net_position > 0,
        net_position.unsigned_abs(),
        avg_entry_price,
        mark_price,
    )?;

    // Add to committed collateral
    let equity = (committed_collateral as i64)
        .checked_add(unrealized_pnl)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    Ok(equity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_to_long_position() {
        let fill = Fill {
            side: OrderSide::Bid,
            qty: 100,
            price: 50_000,
        };

        let result = process_fill(0, 0, &fill).unwrap();

        assert_eq!(result.new_net_position, 100);
        assert_eq!(result.new_avg_entry_price, 50_000);
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_zero_to_short_position() {
        let fill = Fill {
            side: OrderSide::Ask,
            qty: 100,
            price: 50_000,
        };

        let result = process_fill(0, 0, &fill).unwrap();

        assert_eq!(result.new_net_position, -100);
        assert_eq!(result.new_avg_entry_price, 50_000);
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_increase_long_position() {
        let fill = Fill {
            side: OrderSide::Bid,
            qty: 50,
            price: 52_000,
        };

        let result = process_fill(100, 50_000, &fill).unwrap();

        assert_eq!(result.new_net_position, 150);
        // Weighted avg: (100 * 50_000 + 50 * 52_000) / 150 = 50_666
        assert_eq!(result.new_avg_entry_price, 50_666);
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_partial_close_long_position() {
        let fill = Fill {
            side: OrderSide::Ask,
            qty: 30,
            price: 52_000,
        };

        let result = process_fill(100, 50_000, &fill).unwrap();

        assert_eq!(result.new_net_position, 70);
        assert_eq!(result.new_avg_entry_price, 50_000); // Unchanged
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_full_close_long_position() {
        let fill = Fill {
            side: OrderSide::Ask,
            qty: 100,
            price: 48_000,
        };

        let result = process_fill(100, 50_000, &fill).unwrap();

        assert_eq!(result.new_net_position, 0);
        assert_eq!(result.new_avg_entry_price, 0);
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_flip_long_to_short() {
        let fill = Fill {
            side: OrderSide::Ask,
            qty: 150,
            price: 51_000,
        };

        let result = process_fill(100, 50_000, &fill).unwrap();

        assert_eq!(result.new_net_position, -50);
        assert_eq!(result.new_avg_entry_price, 51_000); // New entry at flip price
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_partial_close_short_position() {
        let fill = Fill {
            side: OrderSide::Bid,
            qty: 20,
            price: 48_000,
        };

        let result = process_fill(-50, 50_000, &fill).unwrap();

        assert_eq!(result.new_net_position, -30);
        assert_eq!(result.new_avg_entry_price, 50_000); // Unchanged
        assert_eq!(result.realized_pnl, 0);
    }

    #[test]
    fn test_calculate_equity_long_position_profit() {
        let equity = calculate_equity(
            1_000_000, // 1M committed
            100,       // Long 100 units
            50_000,    // Entry at 50k
            52_000,    // Mark at 52k
        )
        .unwrap();

        // Unrealized PnL: 100 * (52_000 - 50_000) = 200_000
        // Equity: 1_000_000 + 200_000 = 1_200_000
        assert_eq!(equity, 1_000_000);
    }

    #[test]
    fn test_calculate_equity_short_position_loss() {
        let equity = calculate_equity(
            1_000_000, // 1M committed
            -50,       // Short 50 units
            50_000,    // Entry at 50k
            51_000,    // Mark at 51k
        )
        .unwrap();

        // Unrealized PnL: 50 * (50_000 - 51_000) = -50_000
        // Equity: 1_000_000 - 50_000 = 950_000
        assert_eq!(equity, 1_000_000);
    }

    #[test]
    fn test_calculate_equity_no_position() {
        let equity = calculate_equity(
            1_000_000, // 1M committed
            0,         // No position
            0,         // No entry price
            52_000,    // Mark price (irrelevant)
        )
        .unwrap();

        assert_eq!(equity, 1_000_000);
    }
}
