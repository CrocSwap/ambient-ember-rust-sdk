use crate::state::order::OrderDetails;
use crate::storage::market_order_log::{
    FillLogDetails, MarketOrderLogStats, OrderLogEntry, OrderUpdateType,
};
use borsh::{BorshDeserialize, BorshSerialize};
use core::mem::{align_of, size_of};
use solana_program::{account_info::AccountInfo, msg, program_error::ProgramError, pubkey::Pubkey};

use crate::storage::market_order_log::EMPTY_FILL_DETAILS;

/// Version constant for future compatibility
pub const ORDER_LOG_VERSION: u8 = 1;

/// Minimum alignment required for safe pointer operations
const MIN_ALIGNMENT: usize = 8;

/// Known zero padding for OrderDetails struct
pub const ORDER_DETAILS_PADDING: usize = 120;

/// Error types specific to zero-copy order log operations
#[derive(Debug, PartialEq)]
pub enum ZeroCopyOrderLogError {
    LogFull,
    AccountTooSmall,
    CorruptedData,
    InvalidAlignment,
    UnsupportedVersion,
    InvalidCapacity,
    InvalidEntrySize,
}

impl From<ZeroCopyOrderLogError> for ProgramError {
    fn from(e: ZeroCopyOrderLogError) -> Self {
        match e {
            ZeroCopyOrderLogError::LogFull => ProgramError::Custom(300),
            ZeroCopyOrderLogError::AccountTooSmall => ProgramError::Custom(301),
            ZeroCopyOrderLogError::CorruptedData => ProgramError::Custom(302),
            ZeroCopyOrderLogError::InvalidAlignment => ProgramError::Custom(303),
            ZeroCopyOrderLogError::UnsupportedVersion => ProgramError::Custom(304),
            ZeroCopyOrderLogError::InvalidCapacity => ProgramError::Custom(305),
            ZeroCopyOrderLogError::InvalidEntrySize => ProgramError::Custom(306),
        }
    }
}

/// Fixed-size header for the market order log
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct MarketOrderLogHeader {
    /// Version for migration compatibility
    pub version: u8,
    /// Alignment padding
    pub _pad1: [u8; 7],
    /// The market ID this log belongs to
    pub market_id: u64,
    /// The page number (for pagination when exceeding max size)
    pub page: u32,
    /// Alignment padding
    pub _pad2: [u8; 4],
    /// Maximum number of entries this log can hold
    pub capacity: u64,
    /// Current number of entries in the log
    pub entry_count: u64,
    /// Size of each serialized entry (for validation)
    pub entry_size: u64,
    /// Future expansion space
    pub _pad3: [u8; 32],
}

const HEADER_SIZE: usize = size_of::<MarketOrderLogHeader>();

/// Calculate the actual serialized size of OrderLogEntry at runtime
/// This ensures we use the same serialization method during both initialization and usage
pub fn get_entry_serialized_size() -> Result<usize, ZeroCopyOrderLogError> {
    use crate::state::order::{OrderOriginator, OrderPrice, OrderTombstone, TriggerCondition};
    use crate::OrderSide;

    // Use a representative OrderDetails that matches what's actually created in handlers
    let representative_order_details = OrderDetails {
        order_id: 1,
        side: OrderSide::Bid,
        qty: 1000000,
        filled_qty: 0,
        price: OrderPrice::Limit(100000),
        origin: OrderOriginator::User(),
        cancel_cond: TriggerCondition::Off(),
        tombstone: OrderTombstone::Open(),
        ..Default::default()
    };

    let representative_fill_details = EMPTY_FILL_DETAILS;

    let dummy_entry = OrderLogEntry {
        user: Pubkey::default(),
        order_id: 0,
        update_type: OrderUpdateType::OrderEntry,
        order_details: representative_order_details,
        fill_details: representative_fill_details,
        slot: 0,
    };

    dummy_entry
        .try_to_vec()
        .map(|v| v.len())
        .map_err(|_| ZeroCopyOrderLogError::InvalidEntrySize)
}

/// Zero-copy view over a market order log PDA's data
pub struct ZeroCopyMarketOrderLog<'a> {
    data: &'a mut [u8],
    entry_size: usize,
}

impl<'a> ZeroCopyMarketOrderLog<'a> {
    /// Initialize account data with proper validation
    pub fn init_in_account(
        account: &AccountInfo<'a>,
        market_id: u64,
        page: u32,
        capacity: u64,
    ) -> Result<(), ProgramError> {
        if capacity == 0 {
            msg!("Error: Capacity cannot be zero");
            return Err(ZeroCopyOrderLogError::InvalidCapacity.into());
        }

        let entry_size = get_entry_serialized_size()?;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let mut data = account.try_borrow_mut_data()?;

        if data.len() < required_size {
            msg!(
                "Error: Account too small. Need {} bytes, have {}",
                required_size,
                data.len()
            );
            return Err(ZeroCopyOrderLogError::AccountTooSmall.into());
        }

        // Validate alignment
        if (data.as_ptr() as usize) % MIN_ALIGNMENT != 0 {
            msg!("Error: Account data not properly aligned");
            return Err(ZeroCopyOrderLogError::InvalidAlignment.into());
        }

        // Zero out the account data
        for b in data.iter_mut() {
            *b = 0;
        }

        // Initialize header
        let header = MarketOrderLogHeader {
            version: ORDER_LOG_VERSION,
            _pad1: [0; 7],
            market_id,
            page,
            _pad2: [0; 4],
            capacity,
            entry_count: 0,
            entry_size: entry_size as u64,
            _pad3: [0; 32],
        };

        // Safe header write with alignment check
        Self::write_header(&mut data, &header)?;

        msg!(
            "Initialized ZeroCopyMarketOrderLog: market_id={}, page={}, capacity={}, entry_size={}",
            market_id,
            page,
            capacity,
            entry_size
        );
        Ok(())
    }

    /// Load a zero-copy view from existing account data
    pub fn load(account: &AccountInfo<'a>) -> Result<Self, ProgramError> {
        let mut data = account.try_borrow_mut_data()?;
        let data: &'a mut [u8] = unsafe { std::mem::transmute(&mut **data) };

        // Validate minimum size
        if data.len() < HEADER_SIZE {
            msg!("Error: Account data too small for header");
            return Err(ZeroCopyOrderLogError::AccountTooSmall.into());
        }

        // Validate alignment
        if (data.as_ptr() as usize) % MIN_ALIGNMENT != 0 {
            msg!("Error: Account data not properly aligned");
            return Err(ZeroCopyOrderLogError::InvalidAlignment.into());
        }

        let header = Self::read_header(data)?;

        // Validate version
        if header.version != ORDER_LOG_VERSION {
            msg!("Error: Unsupported log version: {}", header.version);
            return Err(ZeroCopyOrderLogError::UnsupportedVersion.into());
        }

        // Validate entry size matches current runtime
        let current_entry_size = get_entry_serialized_size()?;
        if header.entry_size != current_entry_size as u64 {
            msg!(
                "Error: Stored entry size {} doesn't match current {}",
                header.entry_size,
                current_entry_size
            );
            return Err(ZeroCopyOrderLogError::InvalidEntrySize.into());
        }

        let entry_size = header.entry_size as usize;

        // Validate account size can hold the capacity
        let required_size = HEADER_SIZE + (header.capacity as usize * entry_size);
        if data.len() < required_size {
            msg!(
                "Error: Account too small for declared capacity. Need {} bytes, have {}",
                required_size,
                data.len()
            );
            return Err(ZeroCopyOrderLogError::AccountTooSmall.into());
        }

        // Additional corruption checks
        if header.entry_count > header.capacity {
            msg!(
                "Error: Entry count {} exceeds capacity {}",
                header.entry_count,
                header.capacity
            );
            return Err(ZeroCopyOrderLogError::CorruptedData.into());
        }

        Ok(Self { data, entry_size })
    }

    /// Safely read header with bounds checking
    fn read_header(data: &[u8]) -> Result<MarketOrderLogHeader, ZeroCopyOrderLogError> {
        if data.len() < HEADER_SIZE {
            return Err(ZeroCopyOrderLogError::AccountTooSmall);
        }

        // Safe aligned read
        let header_ptr = data.as_ptr() as *const MarketOrderLogHeader;
        if (header_ptr as usize) % align_of::<MarketOrderLogHeader>() != 0 {
            return Err(ZeroCopyOrderLogError::InvalidAlignment);
        }

        Ok(unsafe { *header_ptr })
    }

    /// Safely write header with bounds checking
    fn write_header(
        data: &mut [u8],
        header: &MarketOrderLogHeader,
    ) -> Result<(), ZeroCopyOrderLogError> {
        if data.len() < HEADER_SIZE {
            return Err(ZeroCopyOrderLogError::AccountTooSmall);
        }

        let header_ptr = data.as_mut_ptr() as *mut MarketOrderLogHeader;
        if (header_ptr as usize) % align_of::<MarketOrderLogHeader>() != 0 {
            return Err(ZeroCopyOrderLogError::InvalidAlignment);
        }

        unsafe { *header_ptr = *header };
        Ok(())
    }

    /// Get immutable reference to header
    fn header(&self) -> Result<&MarketOrderLogHeader, ZeroCopyOrderLogError> {
        Self::read_header(self.data).map(|_| {
            // Safe: we validated alignment and size in load()
            unsafe { &*(self.data.as_ptr() as *const MarketOrderLogHeader) }
        })
    }

    /// Get mutable reference to header
    fn header_mut(&mut self) -> Result<&mut MarketOrderLogHeader, ZeroCopyOrderLogError> {
        if self.data.len() < HEADER_SIZE {
            return Err(ZeroCopyOrderLogError::AccountTooSmall);
        }

        // Safe: we validated alignment and size in load()
        Ok(unsafe { &mut *(self.data.as_mut_ptr() as *mut MarketOrderLogHeader) })
    }

    /// Get entry data at given index with bounds checking
    fn get_entry_data(&self, idx: u64, size: usize) -> Result<&[u8], ZeroCopyOrderLogError> {
        let header = self.header()?;
        if idx >= header.entry_count {
            return Err(ZeroCopyOrderLogError::CorruptedData);
        }

        let offset = HEADER_SIZE + (idx as usize * self.entry_size);
        let end = offset + size;

        if size > self.entry_size + ORDER_DETAILS_PADDING {
            msg!(
                "Error: Entry size {} exceeds expected size {}",
                size,
                self.entry_size + ORDER_DETAILS_PADDING
            );
            return Err(ZeroCopyOrderLogError::InvalidEntrySize);
        }

        if end > self.data.len() {
            return Err(ZeroCopyOrderLogError::AccountTooSmall);
        }

        Ok(&self.data[offset..end])
    }

    /// Get mutable entry data at given index with bounds checking
    fn get_entry_data_mut(
        &mut self,
        idx: u64,
        size: usize,
    ) -> Result<&mut [u8], ZeroCopyOrderLogError> {
        let header = self.header()?;
        if idx >= header.capacity {
            return Err(ZeroCopyOrderLogError::CorruptedData);
        }

        let offset = HEADER_SIZE + (idx as usize * self.entry_size);
        let end = offset + size;

        if size > self.entry_size + ORDER_DETAILS_PADDING {
            msg!(
                "Error: Entry size {} exceeds expected size {}",
                size,
                self.entry_size + ORDER_DETAILS_PADDING
            );
            return Err(ZeroCopyOrderLogError::InvalidEntrySize);
        }

        if end > self.data.len() {
            return Err(ZeroCopyOrderLogError::AccountTooSmall);
        }

        Ok(&mut self.data[offset..end])
    }

    /// Serialize and write an entry to the given slot
    fn write_entry(
        &mut self,
        idx: u64,
        entry: &OrderLogEntry,
    ) -> Result<(), ZeroCopyOrderLogError> {
        let serialized = entry
            .try_to_vec()
            .map_err(|_| ZeroCopyOrderLogError::InvalidEntrySize)?;

        let entry_data = self.get_entry_data_mut(idx, serialized.len())?;
        entry_data.copy_from_slice(&serialized);
        Ok(())
    }

    /// Read and deserialize an entry from the given slot
    fn read_entry(&self, idx: u64) -> Result<OrderLogEntry, ZeroCopyOrderLogError> {
        let entry_data = self.get_entry_data(idx, self.entry_size + ORDER_DETAILS_PADDING)?;
        let mut entry_slice = entry_data;

        match OrderLogEntry::deserialize(&mut entry_slice) {
            Ok(entry) => Ok(entry),
            Err(_e) => Err(ZeroCopyOrderLogError::CorruptedData),
        }
        //OrderLogEntry::try_from_slice(entry_data).map_err(|_| ZeroCopyOrderLogError::CorruptedData)
    }

    /// Append a new entry to the log (O(1) operation)
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

        self.append_log_entry(entry)
    }

    fn append_log_entry(&mut self, entry: OrderLogEntry) -> Result<(), ProgramError> {
        let header = self.header()?;

        // Check if log is full
        if header.entry_count >= header.capacity {
            msg!("Market Log is full, cannot append entry: market_id={}, page={}, entry_count={}, capacity={}", header.market_id, header.page, header.entry_count, header.capacity);
            return Err(ZeroCopyOrderLogError::LogFull.into());
        }

        // Write the entry to the next available slot
        let entry_idx = header.entry_count;
        self.write_entry(entry_idx, &entry)?;

        // Update the header (only increment count)
        let header = self.header_mut()?;
        header.entry_count += 1;

        msg!(
            "Appended order log entry: market_id={}, page={}, entry_count={}, user={}, order_id={}, update_type={:?}",
            header.market_id,
            header.page,
            header.entry_count,
            entry.user,
            entry.order_id,
            entry.update_type
        );

        Ok(())
    }

    pub fn append_user_collateral_update(
        &mut self,
        user: Pubkey,
        collateral_snapshot: u64,
        slot: u64,
    ) -> Result<(), ProgramError> {
        let entry = OrderLogEntry::synth_user_collateral_update(user, collateral_snapshot, slot);
        self.append_log_entry(entry)
    }

    /// Check if the log needs reallocation (for account resizing)
    /// For zero-copy logs, only reallocate when approaching capacity limits
    pub fn needs_realloc(&self, current_account_size: usize) -> bool {
        if let Ok(header) = self.header() {
            // Check if we're running out of entry capacity (>90% full)
            let capacity_threshold = (header.capacity * 9) / 10;
            if header.entry_count >= capacity_threshold {
                // We need more capacity - double it
                let new_capacity = header.capacity * 2;
                let required_size = HEADER_SIZE + (new_capacity as usize * self.entry_size);
                required_size > current_account_size
            } else {
                // Still have capacity, no reallocation needed
                false
            }
        } else {
            false
        }
    }

    /// Estimate the required size for the log based on current capacity
    /// For zero-copy logs, this should always return the full capacity size
    pub fn estimated_serialized_size(&self) -> usize {
        if let Ok(header) = self.header() {
            // Return size for current capacity, not just used entries
            HEADER_SIZE + (header.capacity as usize * self.entry_size)
        } else {
            0
        }
    }

    /// Get statistics about the log (zero-copy)
    pub fn get_stats(&self) -> Result<MarketOrderLogStats, ProgramError> {
        let header = self.header()?;

        let mut order_entries = 0;
        let mut cancels = 0;
        let mut fills = 0;
        let mut liquidations = 0;
        let mut close_positions = 0;
        let mut user_collateral_updates = 0;
        let mut other = 0;

        // Iterate through entries without full deserialization
        for idx in 0..header.entry_count {
            let entry = self.read_entry(idx)?;
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

        Ok(MarketOrderLogStats {
            total_entries: header.entry_count,
            order_entries,
            cancels,
            fills,
            liquidations,
            close_positions,
            user_collateral_updates,
            other,
        })
    }

    /// Get basic log info without scanning entries
    pub fn get_basic_info(&self) -> Result<ZeroCopyOrderLogInfo, ProgramError> {
        let header = self.header()?;
        Ok(ZeroCopyOrderLogInfo {
            market_id: header.market_id,
            page: header.page,
            capacity: header.capacity,
            entry_count: header.entry_count,
            entry_size: header.entry_size as usize,
            version: header.version,
        })
    }

    /// Iterate over entries in a range (lazy deserialization)
    pub fn iter_entries_range(
        &self,
        start: u64,
        end: u64,
    ) -> impl Iterator<Item = Result<OrderLogEntry, ZeroCopyOrderLogError>> + '_ {
        let entry_count = self.header().map(|h| h.entry_count).unwrap_or(0);

        let actual_end = std::cmp::min(end, entry_count);
        let actual_start = std::cmp::min(start, actual_end);

        (actual_start..actual_end).map(move |idx| self.read_entry(idx))
    }

    /// Iterate over all entries (lazy deserialization)
    pub fn iter_entries(
        &self,
    ) -> impl Iterator<Item = Result<OrderLogEntry, ZeroCopyOrderLogError>> + '_ {
        let entry_count = self.header().map(|h| h.entry_count).unwrap_or(0);
        self.iter_entries_range(0, entry_count)
    }

    /// Get the current capacity of the log
    pub fn capacity(&self) -> Result<u64, ProgramError> {
        let header = self.header()?;
        Ok(header.capacity)
    }

    /// Update the capacity of the log (used after reallocation)
    pub fn update_capacity(&mut self, new_capacity: u64) -> Result<(), ProgramError> {
        let header = self.header_mut()?;

        // Ensure new capacity is at least as large as current entry count
        if new_capacity < header.entry_count {
            msg!(
                "Error: New capacity {} cannot be less than current entry count {}",
                new_capacity,
                header.entry_count
            );
            return Err(ProgramError::InvalidArgument);
        }

        header.capacity = new_capacity;
        Ok(())
    }
}

/// Basic information about the log without scanning entries
#[derive(Debug)]
pub struct ZeroCopyOrderLogInfo {
    pub market_id: u64,
    pub page: u32,
    pub capacity: u64,
    pub entry_count: u64,
    pub entry_size: usize,
    pub version: u8,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::cma::CmaFillResult;
    use crate::state::order::{
        OrderOriginator, OrderPrice, OrderSide, OrderTombstone, TriggerCondition,
    };

    /// Create a representative OrderDetails that matches the one used in get_entry_serialized_size()
    /// This ensures consistent serialization sizes across tests and runtime
    fn create_representative_order_details() -> OrderDetails {
        OrderDetails {
            order_id: 1,
            side: OrderSide::Bid,
            qty: 1000000,
            filled_qty: 0,
            price: OrderPrice::Limit(100000),
            origin: OrderOriginator::User(),
            cancel_cond: TriggerCondition::Off(),
            tombstone: OrderTombstone::Open(),
            ..Default::default()
        }
    }
    use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

    // Test helper - create account with proper lifetimes
    fn create_test_account_data(size: usize) -> (u64, Vec<u8>) {
        let lamports = 0u64;
        let data = vec![0u8; size];
        (lamports, data)
    }

    fn make_account_info<'a>(lamports: &'a mut u64, data: &'a mut [u8]) -> AccountInfo<'a> {
        static OWNER: Pubkey = Pubkey::new_from_array([0; 32]);
        static KEY: Pubkey = Pubkey::new_from_array([1; 32]);
        AccountInfo::new(&KEY, false, true, lamports, data, &OWNER, false, 0)
    }

    #[test]
    fn test_init_and_basic_operations() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 4u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        // Initialize log
        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();

        // Load log
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();
        let info = log.get_basic_info().unwrap();
        assert_eq!(info.market_id, 42);
        assert_eq!(info.page, 0);
        assert_eq!(info.capacity, capacity);
        assert_eq!(info.entry_count, 0);
        assert_eq!(info.version, ORDER_LOG_VERSION);

        // Create test order details
        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

        // Append entry
        log.append_entry(
            user,
            12345,
            OrderUpdateType::OrderEntry,
            order_details,
            None,
            100000,
        )
        .unwrap();

        let info = log.get_basic_info().unwrap();
        assert_eq!(info.entry_count, 1);

        // Read back the entry
        let entries: Vec<_> = log.iter_entries().collect();
        assert_eq!(entries.len(), 1);
        let entry = entries[0].as_ref().unwrap();
        assert_eq!(entry.user, user);
        assert_eq!(entry.order_id, 12345);
        assert_eq!(entry.update_type, OrderUpdateType::OrderEntry);
    }

    #[test]
    fn test_multiple_entries_and_stats() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 10u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();

        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

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

        let stats = log.get_stats().unwrap();
        assert_eq!(stats.total_entries, 5);
        assert_eq!(stats.order_entries, 2);
        assert_eq!(stats.fills, 1);
        assert_eq!(stats.cancels, 1);
        assert_eq!(stats.liquidations, 1);
        assert_eq!(stats.close_positions, 0);
    }

    #[test]
    fn test_log_full() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 2u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();

        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

        // Fill capacity
        for i in 0..capacity {
            log.append_entry(
                user,
                i,
                OrderUpdateType::OrderEntry,
                order_details.clone(),
                None,
                100000 + i,
            )
            .unwrap();
        }

        // Try to add one more - should fail
        let result = log.append_entry(
            user,
            999,
            OrderUpdateType::OrderEntry,
            order_details,
            None,
            999999,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_range_iteration() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 10u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();

        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

        // Add 5 entries
        for i in 0..5 {
            log.append_entry(
                user,
                i,
                OrderUpdateType::OrderEntry,
                order_details.clone(),
                None,
                100000 + i,
            )
            .unwrap();
        }

        // Test range iteration
        let range_entries: Vec<_> = log.iter_entries_range(1, 4).collect();
        assert_eq!(range_entries.len(), 3);

        for (i, result) in range_entries.iter().enumerate() {
            let entry = result.as_ref().unwrap();
            assert_eq!(entry.order_id, (i + 1) as u64);
        }
    }

    #[test]
    fn test_append_entry_with_fill_details() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 5u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();

        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

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

        // Read back and verify the fill details were correctly recorded
        let entries: Vec<_> = log.iter_entries().collect();
        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        assert_eq!(entry.update_type, OrderUpdateType::Fill);
        assert_eq!(entry.fill_details.price, fill_details.price);
        assert_eq!(entry.fill_details.qty, fill_details.qty);
        assert_eq!(
            entry.fill_details.account.new_net_position,
            fill_details.account.new_net_position
        );
        assert_eq!(
            entry.fill_details.account.old_net_position,
            fill_details.account.old_net_position
        );
        assert_eq!(
            entry.fill_details.account.realized_pnl_banked,
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

        let entries: Vec<_> = log.iter_entries().collect();
        assert_eq!(entries.len(), 2);

        let second_entry = entries[1].as_ref().unwrap();
        assert_eq!(second_entry.fill_details.price, EMPTY_FILL_DETAILS.price);
        assert_eq!(second_entry.fill_details.qty, EMPTY_FILL_DETAILS.qty);
    }

    #[test]
    fn test_append_user_collateral_update() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 5u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();

        let user = Pubkey::new_unique();
        let collateral_snapshot = 1_000_000_000; // 10 USDC
        let slot = 123456;

        // Append user collateral update
        log.append_user_collateral_update(user, collateral_snapshot, slot)
            .unwrap();

        // Read back and verify the entry was correctly created
        let entries: Vec<_> = log.iter_entries().collect();
        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        assert_eq!(entry.user, user);
        assert_eq!(entry.update_type, OrderUpdateType::UserCollateralUpdate);
        assert_eq!(entry.slot, slot);
        assert_eq!(entry.order_id, 0); // Should be 0 for collateral updates
        assert_eq!(entry.fill_details.qty, collateral_snapshot); // Collateral stored in qty field
        assert_eq!(entry.fill_details.price, 0); // Should be 0
        assert_eq!(entry.order_details, OrderDetails::default()); // Should be default
    }

    #[test]
    fn test_get_stats_with_collateral_updates() {
        let entry_size = get_entry_serialized_size().unwrap();
        let capacity = 10u64;
        let required_size = HEADER_SIZE + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        ZeroCopyMarketOrderLog::init_in_account(&account, 42, 0, capacity).unwrap();
        let mut log = ZeroCopyMarketOrderLog::load(&account).unwrap();

        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

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

        let stats = log.get_stats().unwrap();
        assert_eq!(stats.total_entries, 5);
        assert_eq!(stats.order_entries, 1);
        assert_eq!(stats.fills, 1);
        assert_eq!(stats.cancels, 1);
        assert_eq!(stats.user_collateral_updates, 2);
        assert_eq!(stats.liquidations, 0);
        assert_eq!(stats.close_positions, 0);
    }
}
