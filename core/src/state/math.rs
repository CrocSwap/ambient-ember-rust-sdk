use solana_program::program_error::ProgramError;

/// qty_raw (1e-8) × px_raw (1e-6)  →  notional_raw (1e-6)
pub fn mul_qty_px_to_notional(qty: u64, px: u64) -> Result<u64, ProgramError> {
    let product = (qty as u128)
        .checked_mul(px as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    // Scale back to collateral space: divide by 1e8
    let scaled = product / 100_000_000u128;
    if scaled > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(scaled as u64)
}

/// Signed version: qty_signed × px_signed → PnL in collateral raw (1e-6)
pub fn mul_qty_px_signed(qty: i64, px: i64) -> Result<i64, ProgramError> {
    let product = (qty as i128)
        .checked_mul(px as i128)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let scaled = product / 100_000_000i128;
    if scaled > i64::MAX as i128 || scaled < i64::MIN as i128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(scaled as i64)
}
