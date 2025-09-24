use crate::state::order::{
    OrderDetails, OrderOriginator, OrderPrice, OrderSide, OrderTombstone, TriggerCondition,
    TriggerEntrySize,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::msg;

/// Maximum Solana account size (approximately 10MB)
pub const SOLANA_MAX_ACCOUNT_SIZE: usize = 10_485_760;

/// Initial capacity for OrderDetails storage
pub const INITIAL_ORDER_CAPACITY: usize = 10;

/// Growth increment when expanding storage
pub const ORDER_CAPACITY_INCREMENT: usize = 10;

/// Error types for order detail storage operations
#[derive(Debug, PartialEq)]
pub enum OrderDetailStorageError {
    OrderNotFound,
    InvalidOrderId,
    AccountTooSmall,
    InvalidIndex,
}

/// Per-user order details storage with auto-growth and ring buffer fallback
/// Optimized for append-only insertion with tombstone-based lifecycle management
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct OrderDetailStorage {
    /// Current capacity of the orders vector
    pub capacity: usize,
    /// Number of orders ever inserted (used for generating order_id)
    pub total_inserted: usize,
    /// The actual order details storage
    pub orders: Vec<OrderDetails>,
}

impl OrderDetailStorage {
    /// Create new OrderDetailStorage with initial capacity
    pub fn new() -> Self {
        let mut orders = Vec::with_capacity(INITIAL_ORDER_CAPACITY);
        orders.resize(INITIAL_ORDER_CAPACITY, OrderDetails::default());

        Self {
            capacity: INITIAL_ORDER_CAPACITY,
            total_inserted: 0,
            orders,
        }
    }

    /// Insert a new order details with user-provided order_id
    pub fn insert_order(
        &mut self,
        order_id: u64,
        side: OrderSide,
        qty: u64,
        price: OrderPrice,
        current_account_size: usize,
    ) -> Result<(), OrderDetailStorageError> {
        // Check if order_id already exists
        if self.find_order_index(order_id).is_ok() {
            return Err(OrderDetailStorageError::InvalidOrderId);
        }

        // Create the order details with basic fields
        let order_details = OrderDetails {
            order_id,
            side,
            qty,
            filled_qty: 0,
            price,
            origin: OrderOriginator::User(),
            entry_cond: TriggerCondition::Off(),
            entry_cond_size: TriggerEntrySize::PositionSizePercent(0),
            cancel_cond: TriggerCondition::Off(),
            cancel_cond_2: TriggerCondition::Off(),
            cancel_cond_3: TriggerCondition::Off(),
            tombstone: OrderTombstone::Open(),
            event_history: Default::default(),
            builder_tag: Default::default(),
            _pad1: [0; 64],
            _pad2: [0; 32],
            _pad3: [0; 24],
        };

        // Grow if we've filled current capacity
        if self.total_inserted >= self.capacity {
            msg!(
                "OrderDetailStorage growing from {} to {}",
                self.capacity,
                self.capacity + ORDER_CAPACITY_INCREMENT
            );
            // Predict whether a grow would exceed the account's allocated size
            if !self.can_grow_alloc(current_account_size) {
                return Err(OrderDetailStorageError::AccountTooSmall);
            }
            self.grow_storage();
        }

        self.orders[self.total_inserted] = order_details;
        self.total_inserted += 1;
        Ok(())
    }

    /// Returns true if inserting another order would exceed the account's allocated size.
    pub fn needs_resize(&self, current_account_size: usize) -> bool {
        self.total_inserted >= self.capacity && !self.can_grow_alloc(current_account_size)
    }

    /// Check if growing storage by ORDER_CAPACITY_INCREMENT would fit in the allocated account.
    fn can_grow_alloc(&self, current_account_size: usize) -> bool {
        let per_order_size = std::mem::size_of::<OrderDetails>();
        let estimated_current_size = self.estimated_serialized_size();
        let estimated_new_size = estimated_current_size + ORDER_CAPACITY_INCREMENT * per_order_size;
        estimated_new_size <= current_account_size
    }

    /// Grow storage by ORDER_CAPACITY_INCREMENT
    fn grow_storage(&mut self) {
        let new_capacity = self.capacity + ORDER_CAPACITY_INCREMENT;
        self.orders.resize(new_capacity, OrderDetails::default());
        self.capacity = new_capacity;
    }

    /// Switch from normal mode to ring buffer mode
    // (ring-buffer logic removed)

    /// Get order details by order_id
    pub fn get_order(&self, order_id: u64) -> Result<&OrderDetails, OrderDetailStorageError> {
        let index = self.get_order_index(order_id)?;
        Ok(&self.orders[index])
    }

    /// Get mutable order details by order_id
    pub fn get_order_mut(
        &mut self,
        order_id: u64,
    ) -> Result<&mut OrderDetails, OrderDetailStorageError> {
        let index = self.get_order_index(order_id)?;
        Ok(&mut self.orders[index])
    }

    /// Get order details by order_id with a hint for optimization
    pub fn get_order_with_hint(
        &self,
        order_id: u64,
        hint: u32,
    ) -> Result<&OrderDetails, OrderDetailStorageError> {
        let index = self.find_order_index_with_hint(order_id, hint)?;
        Ok(&self.orders[index])
    }

    /// Get mutable order details by order_id with a hint for optimization
    pub fn get_order_mut_with_hint(
        &mut self,
        order_id: u64,
        hint: u32,
    ) -> Result<&mut OrderDetails, OrderDetailStorageError> {
        let index = self.find_order_index_with_hint(order_id, hint)?;
        Ok(&mut self.orders[index])
    }

    /// Find the storage index for a given order_id by searching
    fn find_order_index(&self, order_id: u64) -> Result<usize, OrderDetailStorageError> {
        for i in 0..self.total_inserted {
            if self.orders[i].order_id == order_id {
                return Ok(i);
            }
        }
        Err(OrderDetailStorageError::OrderNotFound)
    }

    /// Get the storage index for a given order_id
    fn get_order_index(&self, order_id: u64) -> Result<usize, OrderDetailStorageError> {
        self.find_order_index(order_id)
    }

    /// Find order index with a hint for optimization
    pub fn find_order_index_with_hint(
        &self,
        order_id: u64,
        hint: u32,
    ) -> Result<usize, OrderDetailStorageError> {
        let hint_idx = hint as usize;

        // First try the hint if it's valid
        if hint_idx < self.total_inserted && self.orders[hint_idx].order_id == order_id {
            return Ok(hint_idx);
        }

        // Fall back to linear search if hint was wrong
        self.find_order_index(order_id)
    }

    /// Update order tombstone (for cancellation, fills, etc.)
    pub fn update_tombstone(
        &mut self,
        order_id: u64,
        tombstone: OrderTombstone,
    ) -> Result<(), OrderDetailStorageError> {
        let order = self.get_order_mut(order_id)?;
        order.tombstone = tombstone;
        Ok(())
    }

    /// Update order tombstone with a hint for optimization
    pub fn update_tombstone_with_hint(
        &mut self,
        order_id: u64,
        tombstone: OrderTombstone,
        hint: u32,
    ) -> Result<(), OrderDetailStorageError> {
        let order = self.get_order_mut_with_hint(order_id, hint)?;
        order.tombstone = tombstone;
        Ok(())
    }

    /// Cancel an order by setting tombstone to UserCancel
    pub fn cancel_order(&mut self, order_id: u64) -> Result<(), OrderDetailStorageError> {
        self.update_tombstone(order_id, OrderTombstone::UserCancel())
    }

    /// Mark an order as filled
    pub fn fill_order(
        &mut self,
        order_id: u64,
        fill_qty: u64,
    ) -> Result<(), OrderDetailStorageError> {
        let order = self.get_order_mut(order_id)?;
        order.filled_qty += fill_qty;

        if order.filled_qty >= order.qty {
            order.tombstone = OrderTombstone::Filled();
        }

        Ok(())
    }

    /// Get all active (non-tombstoned) orders
    pub fn get_active_orders(&self) -> Vec<(u64, &OrderDetails)> {
        self.orders[..self.total_inserted]
            .iter()
            .filter(|order| order.tombstone_is_alive())
            .map(|order| (order.order_id, order))
            .collect()
    }

    /// Get storage statistics
    pub fn stats(&self) -> OrderDetailStorageStats {
        let active_count = self.get_active_orders().len();

        OrderDetailStorageStats {
            capacity: self.capacity,
            total_inserted: self.total_inserted,
            active_orders: active_count,
            is_ring_buffer: false, // No ring buffer in this simplified version
            start_index: 0,        // No start_index in this simplified version
            utilization_pct: if self.capacity > 0 {
                (active_count * 100) / self.capacity
            } else {
                0
            },
        }
    }

    /// Calculate the estimated serialized size of this storage
    pub fn estimated_serialized_size(&self) -> usize {
        // Base struct size plus vector data
        std::mem::size_of::<Self>() + (self.orders.len() * std::mem::size_of::<OrderDetails>())
    }
}

impl Default for OrderDetailStorage {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait for OrderDetails to check tombstone status
trait TombstoneStatus {
    fn tombstone_is_alive(&self) -> bool;
}

impl TombstoneStatus for OrderDetails {
    fn tombstone_is_alive(&self) -> bool {
        matches!(
            self.tombstone,
            OrderTombstone::Open() | OrderTombstone::PreTrigger()
        )
    }
}

/// Storage statistics for monitoring and debugging
#[derive(Debug)]
pub struct OrderDetailStorageStats {
    pub capacity: usize,
    pub total_inserted: usize,
    pub active_orders: usize,
    pub is_ring_buffer: bool,
    pub start_index: usize,
    pub utilization_pct: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_order_price() -> OrderPrice {
        OrderPrice::Limit(100_000) // $100 in micro-dollars
    }

    #[test]
    fn test_new_storage() {
        let storage = OrderDetailStorage::new();
        assert_eq!(storage.capacity, INITIAL_ORDER_CAPACITY);
        assert_eq!(storage.total_inserted, 0);
        assert_eq!(storage.orders.len(), INITIAL_ORDER_CAPACITY);
    }

    #[test]
    fn test_insert_basic() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 1000; // Mock account size
        let order_id = 12345u64;

        storage
            .insert_order(
                order_id,
                OrderSide::Bid,
                50_000, // 0.05 tokens
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        assert_eq!(storage.total_inserted, 1);

        let order = storage.get_order(order_id).unwrap();
        assert_eq!(order.order_id, order_id);
        assert_eq!(order.side, OrderSide::Bid);
        assert_eq!(order.qty, 50_000);
        assert!(matches!(order.tombstone, OrderTombstone::Open()));
    }

    #[test]
    fn test_multiple_inserts() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        // Insert multiple orders with different IDs
        for i in 0..5 {
            let order_id = 1000 + i;
            storage
                .insert_order(
                    order_id,
                    if i % 2 == 0 {
                        OrderSide::Bid
                    } else {
                        OrderSide::Ask
                    },
                    (i + 1) * 10_000,
                    create_test_order_price(),
                    current_size,
                )
                .unwrap();
        }

        assert_eq!(storage.total_inserted, 5);
        assert_eq!(storage.get_active_orders().len(), 5);

        // Verify we can find each order
        for i in 0..5 {
            let order_id = 1000 + i as u64;
            let order = storage.get_order(order_id).unwrap();
            assert_eq!(order.order_id, order_id);
        }
    }

    #[test]
    fn test_order_cancellation() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 1000;
        let order_id = 5555u64;

        storage
            .insert_order(
                order_id,
                OrderSide::Bid,
                100_000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Cancel the order
        storage.cancel_order(order_id).unwrap();

        let order = storage.get_order(order_id).unwrap();
        assert!(matches!(order.tombstone, OrderTombstone::UserCancel()));
        assert_eq!(storage.get_active_orders().len(), 0);
    }

    #[test]
    fn test_order_filling() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 1000;
        let order_id = 7777u64;

        storage
            .insert_order(
                order_id,
                OrderSide::Ask,
                100_000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Partial fill
        storage.fill_order(order_id, 30_000).unwrap();
        let order = storage.get_order(order_id).unwrap();
        assert_eq!(order.filled_qty, 30_000);
        assert!(matches!(order.tombstone, OrderTombstone::Open())); // Still open

        // Complete fill
        storage.fill_order(order_id, 70_000).unwrap();
        let order = storage.get_order(order_id).unwrap();
        assert_eq!(order.filled_qty, 100_000);
        assert!(matches!(order.tombstone, OrderTombstone::Filled())); // Now filled
        assert_eq!(storage.get_active_orders().len(), 0);
    }

    #[test]
    fn test_storage_growth() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 50000; // Large enough account to allow growth

        // Fill initial capacity
        for i in 0..INITIAL_ORDER_CAPACITY {
            let order_id = 10000 + i as u64;
            storage
                .insert_order(
                    order_id,
                    OrderSide::Bid,
                    (i + 1) as u64 * 1000,
                    create_test_order_price(),
                    current_size,
                )
                .unwrap();
        }

        assert_eq!(storage.capacity, INITIAL_ORDER_CAPACITY);

        // Insert one more - should trigger growth
        let order_id = 20000u64;
        storage
            .insert_order(
                order_id,
                OrderSide::Ask,
                5000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        assert_eq!(
            storage.capacity,
            INITIAL_ORDER_CAPACITY + ORDER_CAPACITY_INCREMENT
        );
        assert_eq!(storage.total_inserted, INITIAL_ORDER_CAPACITY + 1);
    }

    #[test]
    fn test_invalid_order_id() {
        let storage = OrderDetailStorage::new();

        let result = storage.get_order(999);
        assert_eq!(result, Err(OrderDetailStorageError::OrderNotFound));
    }

    #[test]
    fn test_serialization_size() {
        let storage = OrderDetailStorage::new();
        let serialized = storage.try_to_vec().unwrap();
        println!(
            "OrderDetailStorage serialized size: {} bytes",
            serialized.len()
        );
        println!(
            "OrderDetails struct size: {} bytes",
            std::mem::size_of::<OrderDetails>()
        );
        println!("INITIAL_ORDER_CAPACITY: {}", INITIAL_ORDER_CAPACITY);

        // The serialized size should be reasonable
        assert!(!serialized.is_empty());
        assert!(serialized.len() < 10_000); // Should be much less than 10KB for 10 orders
    }

    #[test]
    fn test_needs_resize_logic() {
        let mut storage = OrderDetailStorage::new();
        // Before reaching capacity, needs_resize should always be false
        assert!(!storage.needs_resize(0));
        assert!(!storage.needs_resize(usize::MAX));

        // Simulate storage full (total_inserted == capacity)
        storage.total_inserted = storage.capacity;
        let per_order_size = std::mem::size_of::<OrderDetails>();
        let estimated_current_size = storage.estimated_serialized_size();

        // If allocated size is just one byte less than needed for next growth, needs_resize => true
        let too_small = estimated_current_size + per_order_size * ORDER_CAPACITY_INCREMENT - 1;
        assert!(storage.needs_resize(too_small));

        // If allocated size is exactly enough for next growth, needs_resize => false
        let just_enough = estimated_current_size + per_order_size * ORDER_CAPACITY_INCREMENT;
        assert!(!storage.needs_resize(just_enough));
    }

    #[test]
    fn test_find_order_index_with_hint_correct() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        // Insert several orders
        let order_ids = [1001u64, 2002u64, 3003u64, 4004u64];
        for (i, &order_id) in order_ids.iter().enumerate() {
            storage
                .insert_order(
                    order_id,
                    OrderSide::Bid,
                    (i + 1) as u64 * 1000,
                    create_test_order_price(),
                    current_size,
                )
                .unwrap();
        }

        // Test correct hints
        for (expected_index, &order_id) in order_ids.iter().enumerate() {
            let found_index = storage
                .find_order_index_with_hint(order_id, expected_index as u32)
                .unwrap();
            assert_eq!(found_index, expected_index);
        }
    }

    #[test]
    fn test_find_order_index_with_hint_incorrect() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        // Insert several orders
        let order_ids = [1001u64, 2002u64, 3003u64, 4004u64];
        for (i, &order_id) in order_ids.iter().enumerate() {
            storage
                .insert_order(
                    order_id,
                    OrderSide::Bid,
                    (i + 1) as u64 * 1000,
                    create_test_order_price(),
                    current_size,
                )
                .unwrap();
        }

        // Test incorrect hints - should still find the order via fallback
        let found_index = storage.find_order_index_with_hint(2002u64, 0).unwrap(); // Wrong hint
        assert_eq!(found_index, 1); // Should find at index 1

        let found_index = storage.find_order_index_with_hint(1001u64, 3).unwrap(); // Wrong hint
        assert_eq!(found_index, 0); // Should find at index 0
    }

    #[test]
    fn test_find_order_index_with_hint_out_of_bounds() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        let order_id = 1001u64;
        storage
            .insert_order(
                order_id,
                OrderSide::Bid,
                1000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Test out-of-bounds hint - should fall back to linear search
        let found_index = storage.find_order_index_with_hint(order_id, 999).unwrap();
        assert_eq!(found_index, 0);
    }

    #[test]
    fn test_find_order_index_with_hint_nonexistent() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        storage
            .insert_order(
                1001u64,
                OrderSide::Bid,
                1000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Test nonexistent order with any hint
        let result = storage.find_order_index_with_hint(9999u64, 0);
        assert_eq!(result, Err(OrderDetailStorageError::OrderNotFound));
    }

    #[test]
    fn test_get_order_with_hint_correct() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        let order_id = 1234u64;
        storage
            .insert_order(
                order_id,
                OrderSide::Ask,
                5000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Test with correct hint
        let order = storage.get_order_with_hint(order_id, 0).unwrap();
        assert_eq!(order.order_id, order_id);
        assert_eq!(order.side, OrderSide::Ask);
        assert_eq!(order.qty, 5000);
    }

    #[test]
    fn test_get_order_with_hint_incorrect() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        // Insert multiple orders
        storage
            .insert_order(
                1001u64,
                OrderSide::Bid,
                1000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();
        storage
            .insert_order(
                2002u64,
                OrderSide::Ask,
                2000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Test with incorrect hint - should still work via fallback
        let order = storage.get_order_with_hint(2002u64, 0).unwrap(); // Wrong hint (0), correct index is 1
        assert_eq!(order.order_id, 2002u64);
        assert_eq!(order.side, OrderSide::Ask);
    }

    #[test]
    fn test_get_order_mut_with_hint() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        let order_id = 1234u64;
        storage
            .insert_order(
                order_id,
                OrderSide::Bid,
                1000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Test mutable access with hint
        {
            let order = storage.get_order_mut_with_hint(order_id, 0).unwrap();
            order.filled_qty = 500;
        }

        // Verify the change persisted
        let order = storage.get_order(order_id).unwrap();
        assert_eq!(order.filled_qty, 500);
    }

    #[test]
    fn test_update_tombstone_with_hint() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        let order_id = 1234u64;
        storage
            .insert_order(
                order_id,
                OrderSide::Bid,
                1000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Verify order starts as Open
        let order = storage.get_order(order_id).unwrap();
        assert!(matches!(order.tombstone, OrderTombstone::Open()));

        // Update tombstone using hint
        storage
            .update_tombstone_with_hint(order_id, OrderTombstone::UserCancel(), 0)
            .unwrap();

        // Verify tombstone was updated
        let order = storage.get_order(order_id).unwrap();
        assert!(matches!(order.tombstone, OrderTombstone::UserCancel()));
    }

    #[test]
    fn test_update_tombstone_with_hint_incorrect() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 5000;

        // Insert multiple orders
        storage
            .insert_order(
                1001u64,
                OrderSide::Bid,
                1000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();
        storage
            .insert_order(
                2002u64,
                OrderSide::Ask,
                2000,
                create_test_order_price(),
                current_size,
            )
            .unwrap();

        // Update second order with wrong hint
        storage
            .update_tombstone_with_hint(2002u64, OrderTombstone::Filled(), 0)
            .unwrap(); // Wrong hint

        // Verify it still worked via fallback
        let order = storage.get_order(2002u64).unwrap();
        assert!(matches!(order.tombstone, OrderTombstone::Filled()));

        // Verify first order is unchanged
        let order = storage.get_order(1001u64).unwrap();
        assert!(matches!(order.tombstone, OrderTombstone::Open()));
    }

    #[test]
    fn test_hint_methods_performance_characteristics() {
        let mut storage = OrderDetailStorage::new();
        let current_size = 51000;

        // Insert many orders to test performance difference
        let num_orders = 100;
        println!("Inserting {} orders", num_orders);
        for i in 0..num_orders {
            storage
                .insert_order(
                    (i + 1000) as u64,
                    if i % 2 == 0 {
                        OrderSide::Bid
                    } else {
                        OrderSide::Ask
                    },
                    (i + 1) as u64 * 100,
                    create_test_order_price(),
                    current_size,
                )
                .unwrap();
        }

        println!("Inserted {} orders", num_orders);

        // Test that correct hints work for all orders
        for i in 0..num_orders {
            let order_id = (i + 1000) as u64;
            let expected_index = i;

            // Test find_order_index_with_hint
            let found_index = storage
                .find_order_index_with_hint(order_id, expected_index as u32)
                .unwrap();
            assert_eq!(found_index, expected_index);

            // Test get_order_with_hint
            let order = storage
                .get_order_with_hint(order_id, expected_index as u32)
                .unwrap();
            assert_eq!(order.order_id, order_id);
        }

        println!("Tested {} orders", num_orders);

        // Test that wrong hints still work (via fallback)
        let order_id = 1050u64; // Should be at index 50
        let order = storage.get_order_with_hint(order_id, 0).unwrap(); // Wrong hint
        assert_eq!(order.order_id, order_id);

        println!("Tested {} orders", num_orders);
    }
}
