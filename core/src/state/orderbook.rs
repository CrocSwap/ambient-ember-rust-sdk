use solana_program::program_error::ProgramError;

pub const ORDERBOOK_SNAPSHOT_VERSION: u8 = 3;
pub const ORDERBOOK_LEVELS_PER_SIDE: usize = 30;
pub const ORDERBOOK_SIGFIG_COUNT: usize = 5;
pub const ORDERBOOK_SIGFIG_OPTIONS: [u8; ORDERBOOK_SIGFIG_COUNT] = [0, 2, 3, 4, 5];

pub const ORDERBOOK_HEADER_SIZE: usize = 16;
pub const ORDERBOOK_SIGFIG_HEADER_SIZE: usize = 24;
pub const ORDERBOOK_LEVEL_SIZE: usize = 18;
pub const ORDERBOOK_SIDE_SECTION_SIZE: usize = ORDERBOOK_LEVEL_SIZE * ORDERBOOK_LEVELS_PER_SIDE;
pub const ORDERBOOK_SIGFIG_SECTION_SIZE: usize =
    ORDERBOOK_SIGFIG_HEADER_SIZE + (ORDERBOOK_SIDE_SECTION_SIZE * 2);
pub const ORDERBOOK_SNAPSHOT_SIZE: usize =
    ORDERBOOK_HEADER_SIZE + (ORDERBOOK_SIGFIG_SECTION_SIZE * ORDERBOOK_SIGFIG_COUNT);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderBookSide {
    Bid,
    Ask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderBookLevel {
    pub price: i64,
    pub size: u64,
    pub order_count: u16,
}

pub fn sigfig_index(sigfigs: u8) -> Option<usize> {
    ORDERBOOK_SIGFIG_OPTIONS.iter().position(|v| *v == sigfigs)
}

pub fn assert_valid_sigfig(sigfigs: u8) -> Result<usize, ProgramError> {
    sigfig_index(sigfigs).ok_or(ProgramError::InvalidArgument)
}

pub const fn sigfig_section_offset(idx: usize) -> usize {
    ORDERBOOK_HEADER_SIZE + (idx * ORDERBOOK_SIGFIG_SECTION_SIZE)
}

pub const fn sigfig_header_offset(idx: usize) -> usize {
    sigfig_section_offset(idx)
}

pub const fn bids_offset(idx: usize) -> usize {
    sigfig_section_offset(idx) + ORDERBOOK_SIGFIG_HEADER_SIZE
}

pub const fn asks_offset(idx: usize) -> usize {
    bids_offset(idx) + ORDERBOOK_SIDE_SECTION_SIZE
}

pub const fn level_offset(base: usize, level: usize) -> usize {
    base + (level * ORDERBOOK_LEVEL_SIZE)
}

const PRICE_SCALE_DECIMALS: u32 = 6;

pub fn quantize_price_to_sigfigs(price: i64, sigfigs: u8) -> Option<i64> {
    if price <= 0 {
        return None;
    }

    let idx = sigfig_index(sigfigs)?;
    let required = ORDERBOOK_SIGFIG_OPTIONS[idx];

    if required == 0 {
        return Some(price);
    }

    let mut value = i128::from(price);
    let mut removed_scale = 0_u32;

    while removed_scale < PRICE_SCALE_DECIMALS && value % 10 == 0 {
        value /= 10;
        removed_scale += 1;
    }

    let mut tmp = value.abs();
    let mut digits = 0_u32;
    while tmp > 0 {
        tmp /= 10;
        digits += 1;
    }

    if digits <= required as u32 {
        return Some(price);
    }

    let power = digits - required as u32;
    if power > 38 {
        // Prevent overflow when computing 10^power in i128
        return None;
    }

    let factor = 10_i128.pow(power);
    let base = value / factor;
    let remainder = value % factor;
    let half = factor / 2;

    let mut rounded = base;
    if remainder.abs() >= half {
        rounded += 1;
    }

    let mut result = rounded * factor;
    if removed_scale > 0 {
        result *= 10_i128.pow(removed_scale);
    }

    if result < i128::from(i64::MIN) || result > i128::from(i64::MAX) {
        return None;
    }

    Some(result as i64)
}

pub fn zero_fill_side(data: &mut [u8], base_offset: usize) {
    let start = base_offset;
    let end = start + ORDERBOOK_SIDE_SECTION_SIZE;
    data[start..end].fill(0);
}

pub fn zero_fill_sigfig(data: &mut [u8], idx: usize) {
    let start = sigfig_header_offset(idx);
    let end = start + ORDERBOOK_SIGFIG_SECTION_SIZE;
    data[start..end].fill(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigfig_index() {
        for (idx, sigfigs) in ORDERBOOK_SIGFIG_OPTIONS.iter().enumerate() {
            assert_eq!(sigfig_index(*sigfigs), Some(idx));
        }
        assert_eq!(sigfig_index(6), None);
    }

    #[test]
    fn test_section_offsets() {
        for idx in 0..ORDERBOOK_SIGFIG_COUNT {
            let header = sigfig_header_offset(idx);
            let bids = bids_offset(idx);
            let asks = asks_offset(idx);
            assert!(header < bids);
            assert!(bids < asks);
            assert!(asks + ORDERBOOK_SIDE_SECTION_SIZE <= ORDERBOOK_SNAPSHOT_SIZE);
        }
    }

    #[test]
    fn test_quantize_price_to_sigfigs() {
        assert_eq!(
            quantize_price_to_sigfigs(114_590_000_000, 5),
            Some(114_590_000_000)
        );
        assert_eq!(quantize_price_to_sigfigs(123_456_000, 0), Some(123_456_000));
        assert_eq!(quantize_price_to_sigfigs(123_456_000, 5), Some(123_460_000));
        assert_eq!(quantize_price_to_sigfigs(123_456_000, 6), Some(123_456_000));
        assert_eq!(quantize_price_to_sigfigs(9_990, 2), Some(10_000));
        assert_eq!(quantize_price_to_sigfigs(9_990, 3), Some(9_990));
        assert_eq!(quantize_price_to_sigfigs(0, 2), None);
        assert_eq!(quantize_price_to_sigfigs(-10, 2), None);
    }
}
