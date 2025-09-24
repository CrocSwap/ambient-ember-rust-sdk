// core/src/storage/zero_copy_order_storage.rs
// Zero-copy OrderStorage layout for on-chain PDAs, operating directly on the account's byte slice

use solana_program::{account_info::AccountInfo, program_error::ProgramError};
use borsh::{BorshDeserialize, BorshSerialize};
use crate::state::order::OrderMarker;
use core::mem::size_of;

/// Header occupies the first bytes of the PDA
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct OrderStorageHeader {
    pub capacity: u64,
    pub count: u64,
    pub free_head: u64,
    pub next_free: u64,
}

const HEADER_SIZE: usize = size_of::<OrderStorageHeader>();
const MARKER_LEN: usize = size_of::<OrderMarker>();
const SLOT_SIZE: usize = 8 + MARKER_LEN; // 8 bytes for next_free + marker bytes

/// One slot entry: link field + serialized marker data
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SlotEntry {
    pub next_free: u64,
    pub marker_data: [u8; MARKER_LEN],
}

/// Zero-copy view over a PDA's data for free-list OrderStorage
pub struct ZeroCopyOrderStorage<'a> {
    data: &'a mut [u8],
}

impl<'a> ZeroCopyOrderStorage<'a> {
    /// Initialize account data: zero out region and write header
    pub fn init_in_account(
        account: &AccountInfo<'a>,
        capacity: u64,
    ) -> Result<(), ProgramError> {
        let mut data = account.try_borrow_mut_data()?;
        for b in data.iter_mut() { *b = 0; }
        let hdr = OrderStorageHeader { capacity, count: 0, free_head: u64::MAX, next_free: 0 };
        let hdr_bytes = unsafe { core::slice::from_raw_parts(&hdr as *const _ as *const u8, HEADER_SIZE) };
        data[..HEADER_SIZE].copy_from_slice(hdr_bytes);
        Ok(())
    }

    /// Load a zero-copy view from existing account data
    pub fn load(account: &AccountInfo<'a>) -> Result<Self, ProgramError> {
        let data = account.try_borrow_mut_data()?;
        Ok(Self { data })
    }

    fn header(&self) -> &OrderStorageHeader {
        unsafe { &*(self.data.as_ptr() as *const OrderStorageHeader) }
    }
    fn header_mut(&mut self) -> &mut OrderStorageHeader {
        unsafe { &mut *(self.data.as_mut_ptr() as *mut OrderStorageHeader) }
    }

    fn slots(&mut self) -> &mut [SlotEntry] {
        let hdr = self.header();
        let ptr = unsafe { self.data.as_mut_ptr().add(HEADER_SIZE) } as *mut SlotEntry;
        unsafe { core::slice::from_raw_parts_mut(ptr, hdr.capacity as usize) }
    }

    /// Serialize and write a marker into slot index
    fn write_marker(&mut self, idx: u64, marker: &OrderMarker) -> Result<(), ProgramError> {
        let buf = marker.try_to_vec().map_err(|_| ProgramError::BorshIoError("marker write".into()))?;
        if buf.len() != MARKER_LEN { return Err(ProgramError::Custom(2)); }
        let entry = &mut self.slots()[idx as usize];
        entry.marker_data.copy_from_slice(&buf);
        Ok(())
    }

    /// Read and deserialize a marker from slot index
    fn read_marker(&self, idx: u64) -> Result<OrderMarker, ProgramError> {
        let off = HEADER_SIZE + idx as usize * SLOT_SIZE + 8;
        let slice = &self.data[off .. off + MARKER_LEN];
        OrderMarker::try_from_slice(slice).map_err(|_| ProgramError::BorshIoError("marker read".into()))
    }

    /// Insert a new marker, returning its slot index
    pub fn insert(&mut self, marker: &OrderMarker) -> Result<u64, ProgramError> {
        let hdr = self.header_mut();
        let idx = if hdr.free_head != u64::MAX {
            let head = hdr.free_head;
            let entry = &mut self.slots()[head as usize];
            hdr.free_head = entry.next_free;
            head
        } else if hdr.next_free < hdr.capacity {
            let n = hdr.next_free;
            hdr.next_free += 1;
            n
        } else {
            return Err(ProgramError::Custom(0));
        };
        let entry = &mut self.slots()[idx as usize];
        entry.next_free = 0;
        self.write_marker(idx, marker)?;
        hdr.count += 1;
        Ok(idx)
    }

    /// Remove a marker by matching user and order_id, returning it
    pub fn remove(
        &mut self,
        user: &solana_program::pubkey::Pubkey,
        order_id: u64,
    ) -> Result<OrderMarker, ProgramError> {
        let hdr = self.header_mut();
        let limit = hdr.next_free;
        for i in 0..limit {
            let entry = &mut self.slots()[i as usize];
            if entry.next_free == 0 {
                let m = self.read_marker(i)?;
                if m.user == *user && m.order_id == order_id {
                    entry.next_free = hdr.free_head;
                    hdr.free_head = i;
                    hdr.count -= 1;
                    return Ok(m);
                }
            }
        }
        Err(ProgramError::Custom(1))
    }

    /// Find an active marker, returning its slot index
    pub fn find(
        &self,
        user: &solana_program::pubkey::Pubkey,
        order_id: u64,
    ) -> Option<u64> {
        let hdr = self.header();
        for i in 0..hdr.next_free {
            let off = HEADER_SIZE + i as usize * SLOT_SIZE;
            let entry = unsafe { &*(self.data.as_ptr().add(off) as *const SlotEntry) };
            if entry.next_free == 0 {
                if let Ok(m) = OrderMarker::try_from_slice(&entry.marker_data) {
                    if m.user == *user && m.order_id == order_id {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    /// Iterate over all active slots
    pub fn iter_active(&self) -> impl Iterator<Item = (u64, OrderMarker)> + '_ {
        let hdr = *self.header();
        (0..hdr.next_free).filter_map(move |i| {
            let off = HEADER_SIZE + i as usize * SLOT_SIZE;
            let entry = unsafe { &*(self.data.as_ptr().add(off) as *const SlotEntry) };
            if entry.next_free == 0 {
                OrderMarker::try_from_slice(&entry.marker_data).ok().map(|m| (i, m))
            } else {
                None
            }
        })
    }

    /// Capacity and count getters
    pub fn capacity(&self) -> u64 { self.header().capacity }
    pub fn count(&self) -> u64 { self.header().count }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::{account_info::AccountInfo, pubkey::Pubkey};

    #[test]
    fn test_insert_find_remove_iter() {
        let capacity: u64 = 4;
        let mut lamports = 0u64;
        let mut data_vec = vec![0u8; HEADER_SIZE + (capacity as usize) * SLOT_SIZE];
        let mut account = AccountInfo::new(
            &Pubkey::new_unique(),
            false,
            true,
            &mut lamports,
            &mut data_vec[..],
            &Pubkey::default(),
            false,
            0,
        );

        // Initialize storage in account
        ZeroCopyOrderStorage::init_in_account(&account, capacity).unwrap();

        // Load storage view
        let mut storage = ZeroCopyOrderStorage::load(&account).unwrap();
        assert_eq!(storage.capacity(), capacity);
        assert_eq!(storage.count(), 0);

        // Prepare an OrderMarker
        let user_a = Pubkey::new_unique();
        let mut marker = OrderMarker::default();
        marker.user = user_a;
        marker.order_id = 42;

        // Insert the marker
        let idx = storage.insert(&marker).unwrap();
        assert_eq!(idx, 0);
        assert_eq!(storage.count(), 1);

        // Find the marker
        assert_eq!(storage.find(&user_a, 42), Some(idx));

        // iter_active returns exactly our entry
        let active: Vec<_> = storage.iter_active().collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0], (idx, marker.clone()));

        // Remove the marker
        let removed = storage.remove(&user_a, 42).unwrap();
        assert_eq!(removed, marker);
        assert_eq!(storage.count(), 0);
        assert!(storage.find(&user_a, 42).is_none());
    }
} 