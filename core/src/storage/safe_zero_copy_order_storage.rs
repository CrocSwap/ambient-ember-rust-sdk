// core/src/storage/safe_zero_copy_order_storage.rs
// Enhanced zero-copy OrderStorage with safety validations and proper error handling
//
// UNUSED: Retained for future features - market-wide order registry has been removed

use crate::state::order::OrderMarker;
use borsh::{BorshDeserialize, BorshSerialize};
use core::mem::{align_of, size_of};
use solana_program::{account_info::AccountInfo, msg, program_error::ProgramError};

/// Version constant for future compatibility
pub const STORAGE_VERSION: u8 = 1;

/// Minimum alignment required for safe pointer operations
const MIN_ALIGNMENT: usize = 8;

/// Error types specific to zero-copy storage operations
#[derive(Debug, PartialEq)]
pub enum ZeroCopyStorageError {
    StorageFull,
    OrderNotFound,
    InvalidMarkerSize,
    AccountTooSmall,
    CorruptedData,
    InvalidAlignment,
    UnsupportedVersion,
    InvalidCapacity,
}

impl From<ZeroCopyStorageError> for ProgramError {
    fn from(e: ZeroCopyStorageError) -> Self {
        match e {
            ZeroCopyStorageError::StorageFull => ProgramError::Custom(200),
            ZeroCopyStorageError::OrderNotFound => ProgramError::Custom(201),
            ZeroCopyStorageError::InvalidMarkerSize => ProgramError::Custom(202),
            ZeroCopyStorageError::AccountTooSmall => ProgramError::Custom(203),
            ZeroCopyStorageError::CorruptedData => ProgramError::Custom(204),
            ZeroCopyStorageError::InvalidAlignment => ProgramError::Custom(205),
            ZeroCopyStorageError::UnsupportedVersion => ProgramError::Custom(206),
            ZeroCopyStorageError::InvalidCapacity => ProgramError::Custom(207),
        }
    }
}

/// Enhanced header with version and alignment padding
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrderStorageHeader {
    pub version: u8,      // Version for migration compatibility
    pub _pad1: [u8; 7],   // Alignment to 8 bytes
    pub capacity: u64,    // Maximum number of slots
    pub count: u64,       // Current number of active orders
    pub free_head: u64,   // Index of first free slot (u64::MAX if none)
    pub next_free: u64,   // Next slot to allocate if no free slots available
    pub marker_size: u64, // Runtime validation of serialized marker size
    pub _pad2: [u8; 24],  // Future expansion space
}

const HEADER_SIZE: usize = size_of::<OrderStorageHeader>();

/// Calculate actual serialized size of OrderMarker at runtime
fn get_marker_serialized_size() -> Result<usize, ZeroCopyStorageError> {
    let dummy_marker = OrderMarker::default();
    dummy_marker
        .try_to_vec()
        .map(|v| v.len())
        .map_err(|_| ZeroCopyStorageError::InvalidMarkerSize)
}

/// One slot entry: link field + variable-sized serialized marker data
#[repr(C)]
pub struct SlotEntry {
    pub next_free: u64,    // Link to next free slot (0 if occupied)
    pub marker_data: [u8], // Variable-sized marker data
}

impl SlotEntry {
    /// Get the size of a slot entry given the marker size
    pub fn slot_size(marker_size: usize) -> usize {
        8 + marker_size // 8 bytes for next_free + marker bytes
    }
}

/// Safe zero-copy view over a PDA's data for free-list OrderStorage
pub struct SafeZeroCopyOrderStorage<'a> {
    data: &'a mut [u8],
    marker_size: usize,
    slot_size: usize,
}

impl<'a> SafeZeroCopyOrderStorage<'a> {
    /// Initialize account data with proper validation
    pub fn init_in_account(account: &AccountInfo<'a>, capacity: u64) -> Result<(), ProgramError> {
        if capacity == 0 {
            msg!("Error: Capacity cannot be zero");
            return Err(ZeroCopyStorageError::InvalidCapacity.into());
        }

        let marker_size = get_marker_serialized_size()?;
        let slot_size = SlotEntry::slot_size(marker_size);
        let required_size = HEADER_SIZE + (capacity as usize * slot_size);

        let mut data = account.try_borrow_mut_data()?;

        if data.len() < required_size {
            msg!(
                "Error: Account too small. Need {} bytes, have {}",
                required_size,
                data.len()
            );
            return Err(ZeroCopyStorageError::AccountTooSmall.into());
        }

        // Validate alignment
        if (data.as_ptr() as usize) % MIN_ALIGNMENT != 0 {
            msg!("Error: Account data not properly aligned");
            return Err(ZeroCopyStorageError::InvalidAlignment.into());
        }

        // Zero out the account data
        for b in data.iter_mut() {
            *b = 0;
        }

        // Initialize header
        let header = OrderStorageHeader {
            version: STORAGE_VERSION,
            _pad1: [0; 7],
            capacity,
            count: 0,
            free_head: u64::MAX,
            next_free: 0,
            marker_size: marker_size as u64,
            _pad2: [0; 24],
        };

        // Safe header write with alignment check
        Self::write_header(&mut data, &header)?;

        msg!(
            "Initialized SafeZeroCopyOrderStorage: capacity={}, marker_size={}",
            capacity,
            marker_size
        );
        Ok(())
    }

    /// Load a safe zero-copy view from existing account data
    pub fn load(account: &AccountInfo<'a>) -> Result<Self, ProgramError> {
        let mut data = account.try_borrow_mut_data()?;
        let data: &'a mut [u8] = unsafe { std::mem::transmute(&mut **data) };

        // Validate minimum size
        if data.len() < HEADER_SIZE {
            msg!("Error: Account data too small for header");
            return Err(ZeroCopyStorageError::AccountTooSmall.into());
        }

        // Validate alignment
        if (data.as_ptr() as usize) % MIN_ALIGNMENT != 0 {
            msg!("Error: Account data not properly aligned");
            return Err(ZeroCopyStorageError::InvalidAlignment.into());
        }

        let header = Self::read_header(data)?;

        // Validate version
        if header.version != STORAGE_VERSION {
            msg!("Error: Unsupported storage version: {}", header.version);
            return Err(ZeroCopyStorageError::UnsupportedVersion.into());
        }

        // Validate marker size matches current runtime
        let current_marker_size = get_marker_serialized_size()?;
        if header.marker_size != current_marker_size as u64 {
            msg!(
                "Error: Stored marker size {} doesn't match current {}",
                header.marker_size,
                current_marker_size
            );
            return Err(ZeroCopyStorageError::InvalidMarkerSize.into());
        }

        let marker_size = header.marker_size as usize;
        let slot_size = SlotEntry::slot_size(marker_size);

        // Validate account size can hold the capacity
        let required_size = HEADER_SIZE + (header.capacity as usize * slot_size);
        if data.len() < required_size {
            msg!(
                "Error: Account too small for declared capacity. Need {} bytes, have {}",
                required_size,
                data.len()
            );
            return Err(ZeroCopyStorageError::AccountTooSmall.into());
        }

        // Additional corruption checks
        if header.count > header.capacity {
            msg!(
                "Error: Count {} exceeds capacity {}",
                header.count,
                header.capacity
            );
            return Err(ZeroCopyStorageError::CorruptedData.into());
        }

        if header.next_free > header.capacity {
            msg!(
                "Error: next_free {} exceeds capacity {}",
                header.next_free,
                header.capacity
            );
            return Err(ZeroCopyStorageError::CorruptedData.into());
        }

        Ok(Self {
            data,
            marker_size,
            slot_size,
        })
    }

    /// Safely read header with bounds checking
    fn read_header(data: &[u8]) -> Result<OrderStorageHeader, ZeroCopyStorageError> {
        if data.len() < HEADER_SIZE {
            return Err(ZeroCopyStorageError::AccountTooSmall);
        }

        // Safe aligned read
        let header_ptr = data.as_ptr() as *const OrderStorageHeader;
        if (header_ptr as usize) % align_of::<OrderStorageHeader>() != 0 {
            return Err(ZeroCopyStorageError::InvalidAlignment);
        }

        Ok(unsafe { *header_ptr })
    }

    /// Safely write header with bounds checking
    fn write_header(
        data: &mut [u8],
        header: &OrderStorageHeader,
    ) -> Result<(), ZeroCopyStorageError> {
        if data.len() < HEADER_SIZE {
            return Err(ZeroCopyStorageError::AccountTooSmall);
        }

        let header_ptr = data.as_mut_ptr() as *mut OrderStorageHeader;
        if (header_ptr as usize) % align_of::<OrderStorageHeader>() != 0 {
            return Err(ZeroCopyStorageError::InvalidAlignment);
        }

        unsafe { *header_ptr = *header };
        Ok(())
    }

    /// Get immutable reference to header
    fn header(&self) -> Result<&OrderStorageHeader, ZeroCopyStorageError> {
        Self::read_header(self.data).map(|_| {
            // Safe: we validated alignment and size in load()
            unsafe { &*(self.data.as_ptr() as *const OrderStorageHeader) }
        })
    }

    /// Get mutable reference to header
    fn header_mut(&mut self) -> Result<&mut OrderStorageHeader, ZeroCopyStorageError> {
        if self.data.len() < HEADER_SIZE {
            return Err(ZeroCopyStorageError::AccountTooSmall);
        }

        // Safe: we validated alignment and size in load()
        Ok(unsafe { &mut *(self.data.as_mut_ptr() as *mut OrderStorageHeader) })
    }

    /// Get slot data at given index with bounds checking
    fn get_slot_data(&self, idx: u64) -> Result<&[u8], ZeroCopyStorageError> {
        let header = self.header()?;
        if idx >= header.capacity {
            return Err(ZeroCopyStorageError::CorruptedData);
        }

        let offset = HEADER_SIZE + (idx as usize * self.slot_size);
        let end = offset + self.slot_size;

        if end > self.data.len() {
            return Err(ZeroCopyStorageError::AccountTooSmall);
        }

        Ok(&self.data[offset..end])
    }

    /// Get mutable slot data at given index with bounds checking
    fn get_slot_data_mut(&mut self, idx: u64) -> Result<&mut [u8], ZeroCopyStorageError> {
        let header = self.header()?;
        if idx >= header.capacity {
            return Err(ZeroCopyStorageError::CorruptedData);
        }

        let offset = HEADER_SIZE + (idx as usize * self.slot_size);
        let end = offset + self.slot_size;

        if end > self.data.len() {
            return Err(ZeroCopyStorageError::AccountTooSmall);
        }

        Ok(&mut self.data[offset..end])
    }

    /// Read next_free field from slot
    fn read_next_free(&self, idx: u64) -> Result<u64, ZeroCopyStorageError> {
        let slot_data = self.get_slot_data(idx)?;
        let next_free_bytes = &slot_data[0..8];
        Ok(u64::from_le_bytes(next_free_bytes.try_into().unwrap()))
    }

    /// Write next_free field to slot
    fn write_next_free(&mut self, idx: u64, next_free: u64) -> Result<(), ZeroCopyStorageError> {
        let slot_data = self.get_slot_data_mut(idx)?;
        slot_data[0..8].copy_from_slice(&next_free.to_le_bytes());
        Ok(())
    }

    /// Serialize and write a marker into slot index
    fn write_marker(&mut self, idx: u64, marker: &OrderMarker) -> Result<(), ZeroCopyStorageError> {
        let serialized = marker
            .try_to_vec()
            .map_err(|_| ZeroCopyStorageError::InvalidMarkerSize)?;

        let marker_size = self.marker_size; // Copy to avoid borrow conflicts

        if serialized.len() != marker_size {
            msg!(
                "Error: Marker serialized to {} bytes, expected {}",
                serialized.len(),
                marker_size
            );
            return Err(ZeroCopyStorageError::InvalidMarkerSize);
        }

        let slot_data = self.get_slot_data_mut(idx)?;
        slot_data[8..8 + marker_size].copy_from_slice(&serialized);
        Ok(())
    }

    /// Read and deserialize a marker from slot index
    fn read_marker(&self, idx: u64) -> Result<OrderMarker, ZeroCopyStorageError> {
        let slot_data = self.get_slot_data(idx)?;
        let marker_bytes = &slot_data[8..8 + self.marker_size];

        OrderMarker::try_from_slice(marker_bytes).map_err(|_| ZeroCopyStorageError::CorruptedData)
    }

    /// Insert a new marker, returning its slot index
    pub fn insert(&mut self, marker: &OrderMarker) -> Result<u64, ProgramError> {
        // First, determine which slot to use and get the necessary values
        let (slot_idx, new_free_head, new_next_free) = {
            let header = self.header()?;

            if header.free_head != u64::MAX {
                // Reuse free slot
                let idx = header.free_head;
                let next_free = self.read_next_free(idx)?;
                (idx, next_free, header.next_free)
            } else if header.next_free < header.capacity {
                // Use next available slot
                let idx = header.next_free;
                (idx, header.free_head, header.next_free + 1)
            } else {
                return Err(ZeroCopyStorageError::StorageFull.into());
            }
        };

        // Now perform the modifications
        self.write_next_free(slot_idx, 0)?;
        self.write_marker(slot_idx, marker)?;

        // Update header last
        let header = self.header_mut()?;
        header.free_head = new_free_head;
        header.next_free = new_next_free;
        header.count += 1;

        msg!(
            "Inserted order at slot {}, total count: {}",
            slot_idx,
            header.count
        );
        Ok(slot_idx)
    }

    /// Remove a marker by matching user and order_id, returning it
    pub fn remove(
        &mut self,
        user: &solana_program::pubkey::Pubkey,
        order_id: u64,
    ) -> Result<OrderMarker, ProgramError> {
        let search_limit = self.header()?.next_free;

        for idx in 0..search_limit {
            let next_free = self.read_next_free(idx)?;

            // Skip free slots
            if next_free != 0 {
                continue;
            }

            let marker = self.read_marker(idx)?;
            if marker.user == *user && marker.order_id == order_id {
                // Get current free head before modifying
                let current_free_head = self.header()?.free_head;

                // Mark slot as free and add to free list
                self.write_next_free(idx, current_free_head)?;

                // Update header
                let header = self.header_mut()?;
                header.free_head = idx;
                header.count -= 1;

                msg!(
                    "Removed order at slot {}, remaining count: {}",
                    idx,
                    header.count
                );
                return Ok(marker);
            }
        }

        Err(ZeroCopyStorageError::OrderNotFound.into())
    }

    /// Find an active marker, returning its slot index
    pub fn find(
        &self,
        user: &solana_program::pubkey::Pubkey,
        order_id: u64,
    ) -> Result<Option<u64>, ProgramError> {
        let header = self.header()?;

        for idx in 0..header.next_free {
            let next_free = self.read_next_free(idx)?;

            // Skip free slots
            if next_free != 0 {
                continue;
            }

            let marker = self.read_marker(idx)?;
            if marker.user == *user && marker.order_id == order_id {
                return Ok(Some(idx));
            }
        }

        Ok(None)
    }

    /// Find an order with an index hint for optimization
    pub fn find_with_hint(
        &self,
        user: &solana_program::pubkey::Pubkey,
        order_id: u64,
        hint: u32,
    ) -> Result<Option<u64>, ProgramError> {
        let header = self.header()?;
        let hint_idx = hint as u64;

        // First try the hint if it's valid
        if hint_idx < header.next_free {
            let next_free = self.read_next_free(hint_idx)?;

            // Check if slot is occupied
            if next_free == 0 {
                let marker = self.read_marker(hint_idx)?;
                if marker.user == *user && marker.order_id == order_id {
                    msg!("Order found at hint index {}", hint);
                    return Ok(Some(hint_idx));
                }
            }
        }

        // Fall back to linear search if hint was wrong
        msg!("Hint {} was incorrect, falling back to linear search", hint);
        self.find(user, order_id)
    }

    /// Remove an order with an index hint for optimization
    pub fn remove_with_hint(
        &mut self,
        user: &solana_program::pubkey::Pubkey,
        order_id: u64,
        hint: u32,
    ) -> Result<OrderMarker, ProgramError> {
        let hint_idx = hint as u64;

        // First check if the hint is correct
        if let Some(found_idx) = self.find_with_hint(user, order_id, hint)? {
            // If we found it at the hint index, remove it directly
            if found_idx == hint_idx {
                msg!("Using hint {} for removal", hint);
            }

            // Get the marker before modifying
            let marker = self.read_marker(found_idx)?;

            // Get current free head before modifying
            let current_free_head = self.header()?.free_head;

            // Mark slot as free and add to free list
            self.write_next_free(found_idx, current_free_head)?;

            // Update header
            let header = self.header_mut()?;
            header.free_head = found_idx;
            header.count -= 1;

            msg!(
                "Removed order at slot {}, remaining count: {}",
                found_idx,
                header.count
            );
            return Ok(marker);
        }

        Err(ZeroCopyStorageError::OrderNotFound.into())
    }

    /// Iterate over all active slots with proper error handling
    pub fn iter_active(
        &self,
    ) -> impl Iterator<Item = Result<(u64, OrderMarker), ZeroCopyStorageError>> + '_ {
        let header = match self.header() {
            Ok(h) => *h,
            Err(e) => return Either::Left(std::iter::once(Err(e))),
        };

        Either::Right(
            (0..header.next_free)
                .map(move |idx| {
                    let next_free = self.read_next_free(idx)?;

                    if next_free != 0 {
                        // This is a free slot, skip it
                        return Err(ZeroCopyStorageError::OrderNotFound); // Will be filtered out
                    }

                    let marker = self.read_marker(idx)?;
                    Ok((idx, marker))
                })
                .filter_map(|result| match result {
                    Ok(value) => Some(Ok(value)),
                    Err(ZeroCopyStorageError::OrderNotFound) => None, // Skip free slots
                    Err(e) => Some(Err(e)),                           // Propagate other errors
                }),
        )
    }

    /// Get storage statistics
    pub fn stats(&self) -> Result<SafeZeroCopyStorageStats, ProgramError> {
        let header = self.header()?;

        Ok(SafeZeroCopyStorageStats {
            version: header.version,
            capacity: header.capacity,
            count: header.count,
            next_free: header.next_free,
            free_head: header.free_head,
            marker_size: header.marker_size as usize,
            slot_size: self.slot_size,
            utilization_pct: if header.capacity > 0 {
                (header.count * 100) / header.capacity
            } else {
                0
            },
        })
    }

    /// Validate storage integrity - useful for debugging
    pub fn validate_integrity(&self) -> Result<(), ProgramError> {
        let header = self.header()?;

        // Check basic constraints
        if header.count > header.capacity {
            return Err(ZeroCopyStorageError::CorruptedData.into());
        }

        if header.next_free > header.capacity {
            return Err(ZeroCopyStorageError::CorruptedData.into());
        }

        // Validate free list doesn't have cycles (basic check)
        let mut visited_free_slots = std::collections::HashSet::new();
        let mut current_free = header.free_head;

        while current_free != u64::MAX {
            if !visited_free_slots.insert(current_free) {
                msg!(
                    "Error: Cycle detected in free list at slot {}",
                    current_free
                );
                return Err(ZeroCopyStorageError::CorruptedData.into());
            }

            if current_free >= header.capacity {
                msg!("Error: Free list contains invalid slot {}", current_free);
                return Err(ZeroCopyStorageError::CorruptedData.into());
            }

            current_free = self.read_next_free(current_free)?;
        }

        msg!("Storage integrity validation passed");
        Ok(())
    }
}

/// Helper enum for iterator implementation
enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<L, R> Iterator for Either<L, R>
where
    L: Iterator,
    R: Iterator<Item = L::Item>,
{
    type Item = L::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Either::Left(iter) => iter.next(),
            Either::Right(iter) => iter.next(),
        }
    }
}

/// Storage statistics for monitoring and debugging
#[derive(Debug)]
pub struct SafeZeroCopyStorageStats {
    pub version: u8,
    pub capacity: u64,
    pub count: u64,
    pub next_free: u64,
    pub free_head: u64,
    pub marker_size: usize,
    pub slot_size: usize,
    pub utilization_pct: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let marker_size = get_marker_serialized_size().unwrap();
        let slot_size = SlotEntry::slot_size(marker_size);
        let capacity = 4u64;
        let required_size = HEADER_SIZE + (capacity as usize * slot_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        // Initialize storage
        SafeZeroCopyOrderStorage::init_in_account(&account, capacity).unwrap();

        // Load storage
        let mut storage = SafeZeroCopyOrderStorage::load(&account).unwrap();
        let stats = storage.stats().unwrap();
        assert_eq!(stats.capacity, capacity);
        assert_eq!(stats.count, 0);
        assert_eq!(stats.version, STORAGE_VERSION);

        // Create test marker
        let user = Pubkey::new_unique();
        let mut marker = OrderMarker::default();
        marker.user = user;
        marker.order_id = 42;

        // Insert marker
        let slot_idx = storage.insert(&marker).unwrap();
        assert_eq!(slot_idx, 0);
        assert_eq!(storage.stats().unwrap().count, 1);

        // Find marker
        let found_idx = storage.find(&user, 42).unwrap();
        assert_eq!(found_idx, Some(slot_idx));

        // Remove marker
        let removed = storage.remove(&user, 42).unwrap();
        assert_eq!(removed.user, user);
        assert_eq!(removed.order_id, 42);
        assert_eq!(storage.stats().unwrap().count, 0);

        // Validate integrity
        storage.validate_integrity().unwrap();
    }

    #[test]
    fn test_account_too_small() {
        let (mut lamports, mut data) = create_test_account_data(10); // Too small
        let account = make_account_info(&mut lamports, &mut data);

        let result = SafeZeroCopyOrderStorage::init_in_account(&account, 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_storage_full() {
        let marker_size = get_marker_serialized_size().unwrap();
        let slot_size = SlotEntry::slot_size(marker_size);
        let capacity = 2u64;
        let required_size = HEADER_SIZE + (capacity as usize * slot_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        SafeZeroCopyOrderStorage::init_in_account(&account, capacity).unwrap();
        let mut storage = SafeZeroCopyOrderStorage::load(&account).unwrap();

        // Fill capacity
        for i in 0..capacity {
            let mut marker = OrderMarker::default();
            marker.user = Pubkey::new_unique();
            marker.order_id = i;
            storage.insert(&marker).unwrap();
        }

        // Try to insert one more - should fail
        let mut marker = OrderMarker::default();
        marker.user = Pubkey::new_unique();
        marker.order_id = 999;
        let result = storage.insert(&marker);
        assert!(result.is_err());
    }

    #[test]
    fn test_find_with_hint_correct() {
        let marker_size = get_marker_serialized_size().unwrap();
        let slot_size = SlotEntry::slot_size(marker_size);
        let capacity = 10u64;
        let required_size = HEADER_SIZE + (capacity as usize * slot_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        SafeZeroCopyOrderStorage::init_in_account(&account, capacity).unwrap();
        let mut storage = SafeZeroCopyOrderStorage::load(&account).unwrap();

        // Insert some orders
        let user1 = Pubkey::new_unique();
        let user2 = Pubkey::new_unique();

        let marker1 = OrderMarker::new(user1, 100);
        let slot1 = storage.insert(&marker1).unwrap();

        let marker2 = OrderMarker::new(user2, 200);
        let slot2 = storage.insert(&marker2).unwrap();

        let marker3 = OrderMarker::new(user1, 300);
        let slot3 = storage.insert(&marker3).unwrap();

        // Test find_with_hint with correct hints
        assert_eq!(
            storage.find_with_hint(&user1, 100, slot1 as u32).unwrap(),
            Some(slot1)
        );
        assert_eq!(
            storage.find_with_hint(&user2, 200, slot2 as u32).unwrap(),
            Some(slot2)
        );
        assert_eq!(
            storage.find_with_hint(&user1, 300, slot3 as u32).unwrap(),
            Some(slot3)
        );
    }

    #[test]
    fn test_find_with_hint_incorrect() {
        let marker_size = get_marker_serialized_size().unwrap();
        let slot_size = SlotEntry::slot_size(marker_size);
        let capacity = 10u64;
        let required_size = HEADER_SIZE + (capacity as usize * slot_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        SafeZeroCopyOrderStorage::init_in_account(&account, capacity).unwrap();
        let mut storage = SafeZeroCopyOrderStorage::load(&account).unwrap();

        // Insert some orders
        let user = Pubkey::new_unique();

        let marker1 = OrderMarker::new(user, 100);
        let slot1 = storage.insert(&marker1).unwrap();

        let marker2 = OrderMarker::new(user, 200);
        let slot2 = storage.insert(&marker2).unwrap();

        // Test find_with_hint with incorrect hints - should fall back to linear search
        assert_eq!(
            storage.find_with_hint(&user, 100, slot2 as u32).unwrap(),
            Some(slot1)
        );
        assert_eq!(
            storage.find_with_hint(&user, 200, slot1 as u32).unwrap(),
            Some(slot2)
        );

        // Test with out-of-range hint
        assert_eq!(
            storage.find_with_hint(&user, 100, 999).unwrap(),
            Some(slot1)
        );

        // Test with non-existent order
        assert_eq!(
            storage.find_with_hint(&user, 999, slot1 as u32).unwrap(),
            None
        );
    }

    #[test]
    fn test_remove_with_hint() {
        let marker_size = get_marker_serialized_size().unwrap();
        let slot_size = SlotEntry::slot_size(marker_size);
        let capacity = 10u64;
        let required_size = HEADER_SIZE + (capacity as usize * slot_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        SafeZeroCopyOrderStorage::init_in_account(&account, capacity).unwrap();
        let mut storage = SafeZeroCopyOrderStorage::load(&account).unwrap();

        // Insert some orders
        let user = Pubkey::new_unique();

        let marker1 = OrderMarker::new(user, 100);
        let slot1 = storage.insert(&marker1).unwrap();

        let marker2 = OrderMarker::new(user, 200);
        let slot2 = storage.insert(&marker2).unwrap();

        // Test remove_with_hint with correct hint
        let removed = storage.remove_with_hint(&user, 100, slot1 as u32).unwrap();
        assert_eq!(removed.order_id, 100);
        assert_eq!(storage.stats().unwrap().count, 1);

        // Test remove_with_hint with incorrect hint - should still work
        let removed = storage.remove_with_hint(&user, 200, slot1 as u32).unwrap();
        assert_eq!(removed.order_id, 200);
        assert_eq!(storage.stats().unwrap().count, 0);

        // Test remove non-existent order
        let result = storage.remove_with_hint(&user, 999, 0);
        assert!(result.is_err());
    }
}
