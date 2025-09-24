use crate::state::order::OrderMarker;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Error types for order storage operations
#[derive(Debug, PartialEq)]
pub enum OrderStorageError {
    StorageFull,
    OrderNotFound,
    DuplicateOrder,
    InvalidCapacity,
    InvalidIndex,
}

/// Free list-based order storage for O(1) insert/remove operations
/// Uses a pre-allocated Vec<Option<OrderMarker>> with a stack of free slots
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct OrderStorage {
    /// Maximum number of orders this storage can hold
    pub capacity: usize,
    /// Current number of active orders
    pub count: usize,
    /// Stack of free slot indices for O(1) insertion
    pub free_slots: Vec<usize>,
    /// Next slot to use if free_slots is empty (grows storage)
    pub next_free_slot: usize,
    /// Main storage array - None indicates free slot
    pub orders: Vec<Option<OrderMarker>>,
}

impl OrderStorage {
    /// Create new OrderStorage with specified initial capacity
    pub fn new(capacity: usize) -> Self {
        let mut orders = Vec::with_capacity(capacity);
        orders.resize(capacity, None);

        // Initialize empty free_slots (will be filled as orders are removed)
        let free_slots: Vec<usize> = Vec::new();

        Self {
            capacity,
            count: 0,
            free_slots,
            next_free_slot: 0,
            orders,
        }
    }

    /// Insert a new order, returning the slot index where it was stored
    /// O(1) operation using free list
    ///
    /// @dev Note that we don't check for duplicates here, because duplicate checks are
    ///      handled downstream in the user-specific OrderDetailsRegsitry. However, be
    ///      aware that we're dependent on that logic always being called in the same
    ///      transaction and reverting the whole transactions. Be aware if it changes
    pub fn insert(&mut self, order: OrderMarker) -> Result<usize, OrderStorageError> {
        let slot_index = if let Some(free_index) = self.free_slots.pop() {
            // Reuse a previously freed slot
            free_index
        } else if self.next_free_slot < self.capacity {
            // Use next available slot within capacity
            let index = self.next_free_slot;
            self.next_free_slot += 1;
            index
        } else {
            return Err(OrderStorageError::StorageFull);
        };

        self.orders[slot_index] = Some(order);
        self.count += 1;

        Ok(slot_index)
    }

    /// Remove an order by owner and order_id
    /// Returns the removed order if found
    /// O(n) operation due to linear scan, but O(1) removal once found
    pub fn remove(
        &mut self,
        owner: &Pubkey,
        order_id: u64,
        hint: Option<usize>,
    ) -> Result<OrderMarker, OrderStorageError> {
        let slot_index = self
            .find_by_owner_and_id(owner, order_id, hint)
            .ok_or(OrderStorageError::OrderNotFound)?;

        let order = self.orders[slot_index]
            .take()
            .ok_or(OrderStorageError::OrderNotFound)?;

        // Add slot back to free list for reuse
        self.free_slots.push(slot_index);
        self.count -= 1;

        Ok(order)
    }

    /// Find order slot by owner and order_id
    /// Uses hint if provided for O(1) lookup, otherwise linear scan
    pub fn find_by_owner_and_id(
        &self,
        owner: &Pubkey,
        order_id: u64,
        hint: Option<usize>,
    ) -> Option<usize> {
        // Try hint first if provided
        if let Some(hint_index) = hint {
            if hint_index < self.orders.len() {
                if let Some(ref order) = self.orders[hint_index] {
                    if order.user == *owner && order.order_id == order_id {
                        return Some(hint_index);
                    }
                }
            }
        }

        // Linear scan as fallback
        for (index, slot) in self.orders.iter().enumerate() {
            if let Some(ref order) = slot {
                if order.user == *owner && order.order_id == order_id {
                    return Some(index);
                }
            }
        }

        None
    }

    /// Get order by slot index
    pub fn get(&self, index: usize) -> Result<&OrderMarker, OrderStorageError> {
        self.orders
            .get(index)
            .and_then(|slot| slot.as_ref())
            .ok_or(OrderStorageError::InvalidIndex)
    }

    /// Get mutable order by slot index
    pub fn get_mut(&mut self, index: usize) -> Result<&mut OrderMarker, OrderStorageError> {
        self.orders
            .get_mut(index)
            .and_then(|slot| slot.as_mut())
            .ok_or(OrderStorageError::InvalidIndex)
    }

    /// Resize storage capacity (grow only for safety)
    pub fn resize(&mut self, new_capacity: usize) -> Result<(), OrderStorageError> {
        if new_capacity < self.capacity {
            return Err(OrderStorageError::InvalidCapacity);
        }

        // Grow the orders vec to new capacity
        self.orders.resize(new_capacity, None);
        self.capacity = new_capacity;

        Ok(())
    }

    /// Get storage statistics
    pub fn stats(&self) -> OrderStorageStats {
        OrderStorageStats {
            capacity: self.capacity,
            count: self.count,
            free_slots: self.free_slots.len(),
            utilization_pct: if self.capacity > 0 {
                (self.count * 100) / self.capacity
            } else {
                0
            },
        }
    }

    /// Iterator over all active orders with their slot indices
    pub fn iter_active(&self) -> impl Iterator<Item = (usize, &OrderMarker)> {
        self.orders
            .iter()
            .enumerate()
            .filter_map(|(index, slot)| slot.as_ref().map(|order| (index, order)))
    }

    /// Iterator over all active orders for a specific user
    pub fn iter_user_orders<'a>(
        &'a self,
        user: &'a Pubkey,
    ) -> impl Iterator<Item = (usize, &'a OrderMarker)> + 'a {
        self.iter_active()
            .filter(move |(_, order)| order.user == *user)
    }
}

/// Storage statistics
#[derive(Debug)]
pub struct OrderStorageStats {
    pub capacity: usize,
    pub count: usize,
    pub free_slots: usize,
    pub utilization_pct: usize,
}

impl Default for OrderStorage {
    fn default() -> Self {
        Self::new(50_000) // Default to 50k capacity as requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    fn create_test_order(user: Pubkey, order_id: u64) -> OrderMarker {
        OrderMarker {
            user,
            order_id,
            order_version: 1,
            order_page: 100,
            _pad1: [0; 2],
        }
    }

    #[test]
    fn test_new_storage() {
        let storage = OrderStorage::new(100);
        assert_eq!(storage.capacity, 100);
        assert_eq!(storage.count, 0);
        assert_eq!(storage.free_slots.len(), 0); // Should start empty
        assert_eq!(storage.next_free_slot, 0);
    }

    #[test]
    fn test_insert_and_find() {
        let mut storage = OrderStorage::new(10);
        let user = Pubkey::new_unique();
        let order = create_test_order(user, 123);

        let index = storage.insert(order.clone()).unwrap();
        assert_eq!(storage.count, 1);

        let found_index = storage.find_by_owner_and_id(&user, 123, None).unwrap();
        assert_eq!(index, found_index);
    }

    #[test]
    fn test_remove() {
        let mut storage = OrderStorage::new(10);
        let user = Pubkey::new_unique();
        let order = create_test_order(user, 123);

        storage.insert(order.clone()).unwrap();
        assert_eq!(storage.count, 1);

        let removed = storage.remove(&user, 123, None).unwrap();
        assert_eq!(removed.order_id, 123);
        assert_eq!(storage.count, 0);

        // Should not be findable anymore
        assert!(storage.find_by_owner_and_id(&user, 123, None).is_none());
    }

    #[test]
    fn test_hint_lookup() {
        let mut storage = OrderStorage::new(10);
        let user = Pubkey::new_unique();
        let order = create_test_order(user, 123);

        let index = storage.insert(order).unwrap();

        // Should find with correct hint
        let found = storage.find_by_owner_and_id(&user, 123, Some(index));
        assert_eq!(found, Some(index));

        // Should still find with wrong hint (fallback to linear scan)
        let found = storage.find_by_owner_and_id(&user, 123, Some(999));
        assert_eq!(found, Some(index));
    }

    #[test]
    fn test_resize() {
        let mut storage = OrderStorage::new(10);
        assert_eq!(storage.capacity, 10);

        storage.resize(20).unwrap();
        assert_eq!(storage.capacity, 20);

        // Should not allow shrinking
        let result = storage.resize(5);
        assert_eq!(result, Err(OrderStorageError::InvalidCapacity));
    }

    #[test]
    fn test_storage_full() {
        let mut storage = OrderStorage::new(2);
        let user1 = Pubkey::new_unique();
        let user2 = Pubkey::new_unique();
        let user3 = Pubkey::new_unique();

        storage.insert(create_test_order(user1, 1)).unwrap();
        storage.insert(create_test_order(user2, 2)).unwrap();

        let result = storage.insert(create_test_order(user3, 3));
        assert_eq!(result, Err(OrderStorageError::StorageFull));
    }
}
