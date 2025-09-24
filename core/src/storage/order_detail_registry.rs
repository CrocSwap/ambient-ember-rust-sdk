use crate::state::order::{OrderPrice, OrderSide, OrderTombstone};
use crate::storage::order_detail_storage::{
    OrderDetailStorage, OrderDetailStorageError, INITIAL_ORDER_CAPACITY,
};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, msg, program_error::ProgramError};

/// Custom error codes for OrderDetails PDA operations
const ERROR_NEEDS_RESIZE: u32 = 200;
const ERROR_ORDER_NOT_FOUND: u32 = 111;
const ERROR_INVALID_ORDER_ID: u32 = 112;
const ERROR_ACCOUNT_TOO_SMALL: u32 = 113;
const ERROR_INVALID_INDEX: u32 = 115;

/// Helper functions for working with OrderDetailStorage in per-user OrderDetails PDAs
impl OrderDetailStorage {
    /// Initialize a new OrderDetails PDA for a user with initial capacity
    /// Should be called when creating the PDA for the first time
    pub fn init_in_account(order_details_account: &AccountInfo) -> Result<(), ProgramError> {
        if !order_details_account.data_is_empty() {
            msg!("Error: OrderDetails account already initialized");
            return Err(ProgramError::AccountAlreadyInitialized);
        }

        let storage = OrderDetailStorage::new();
        let serialized = storage.try_to_vec().map_err(|_| {
            ProgramError::BorshIoError("Failed to serialize OrderDetailStorage".to_string())
        })?;

        // Check if account is large enough for initial allocation
        if order_details_account.data_len() < serialized.len() {
            msg!(
                "Error: OrderDetails account too small. Need {} bytes, have {}",
                serialized.len(),
                order_details_account.data_len()
            );
            return Err(ProgramError::AccountDataTooSmall);
        }

        let mut data = order_details_account.try_borrow_mut_data()?;
        data[..serialized.len()].copy_from_slice(&serialized);

        msg!(
            "OrderDetails initialized with {} capacity for user",
            INITIAL_ORDER_CAPACITY
        );
        Ok(())
    }

    /// Load OrderDetailStorage from OrderDetails PDA
    pub fn load_from_account(order_details_account: &AccountInfo) -> Result<Self, ProgramError> {
        if order_details_account.data_is_empty() {
            msg!("Error: OrderDetails account not initialized");
            return Err(ProgramError::UninitializedAccount);
        }

        let data = order_details_account.try_borrow_data()?;

        // Try to deserialize - Borsh will only read the exact bytes it needs
        let mut data_slice = &data[..];
        OrderDetailStorage::deserialize(&mut data_slice).map_err(|e| {
            msg!("Failed to deserialize OrderDetailStorage: {}", e);
            ProgramError::BorshIoError("Failed to deserialize OrderDetailStorage".to_string())
        })
    }

    /// Save OrderDetailStorage back to OrderDetails PDA
    /// Handles account resizing if needed for growth
    pub fn save_to_account(&self, order_details_account: &AccountInfo) -> Result<(), ProgramError> {
        let serialized = self.try_to_vec().map_err(|_| {
            ProgramError::BorshIoError("Failed to serialize OrderDetailStorage".to_string())
        })?;

        let current_data_len = order_details_account.data_len();

        if current_data_len < serialized.len() {
            // Account needs to grow - this would require reallocation
            // In a real implementation, this would need special handling with system program calls
            // For now, we'll return an error indicating the account needs manual resizing
            msg!(
                "Error: OrderDetails account needs resizing. Current: {}, needed: {}",
                current_data_len,
                serialized.len()
            );
            return Err(ProgramError::Custom(ERROR_NEEDS_RESIZE)); // Custom error for account resizing needed
        }

        let mut data = order_details_account.try_borrow_mut_data()?;
        data[..serialized.len()].copy_from_slice(&serialized);

        Ok(())
    }

    /// Atomic operation: Load -> Modify -> Save
    /// Use this for operations that need to modify the storage
    pub fn with_mut_storage<F, R>(
        order_details_account: &AccountInfo,
        f: F,
    ) -> Result<R, ProgramError>
    where
        F: FnOnce(&mut OrderDetailStorage) -> Result<R, OrderDetailStorageError>,
    {
        let mut storage = Self::load_from_account(order_details_account)?;

        let result = f(&mut storage).map_err(|e| {
            msg!("OrderDetailStorage operation failed: {:?}", e);
            match e {
                OrderDetailStorageError::OrderNotFound => {
                    ProgramError::Custom(ERROR_ORDER_NOT_FOUND)
                }
                OrderDetailStorageError::InvalidOrderId => {
                    ProgramError::Custom(ERROR_INVALID_ORDER_ID)
                }
                OrderDetailStorageError::AccountTooSmall => {
                    ProgramError::Custom(ERROR_ACCOUNT_TOO_SMALL)
                }
                OrderDetailStorageError::InvalidIndex => ProgramError::Custom(ERROR_INVALID_INDEX),
            }
        })?;

        msg!("Saving OrderDetailStorage back to account");
        storage.save_to_account(order_details_account)?;
        msg!("Successfully saved OrderDetailStorage");
        Ok(result)
    }

    /// Read-only operation: Load -> Read
    /// Use this for operations that only need to read the storage
    pub fn with_storage<F, R>(order_details_account: &AccountInfo, f: F) -> Result<R, ProgramError>
    where
        F: FnOnce(&OrderDetailStorage) -> Result<R, OrderDetailStorageError>,
    {
        let storage = Self::load_from_account(order_details_account)?;

        f(&storage).map_err(|e| {
            msg!("OrderDetailStorage operation failed: {:?}", e);
            match e {
                OrderDetailStorageError::OrderNotFound => {
                    ProgramError::Custom(ERROR_ORDER_NOT_FOUND)
                }
                OrderDetailStorageError::InvalidOrderId => {
                    ProgramError::Custom(ERROR_INVALID_ORDER_ID)
                }
                OrderDetailStorageError::AccountTooSmall => {
                    ProgramError::Custom(ERROR_ACCOUNT_TOO_SMALL)
                }
                OrderDetailStorageError::InvalidIndex => ProgramError::Custom(ERROR_INVALID_INDEX),
            }
        })
    }
}

/// Convenience functions for common order detail operations
pub mod order_detail_ops {
    use super::*;

    /// Place a new order in the user's OrderDetails storage
    pub fn place_order_detail(
        order_details_account: &AccountInfo,
        order_id: u64,
        side: OrderSide,
        qty: u64,
        price: OrderPrice,
    ) -> Result<(), ProgramError> {
        let current_account_size = order_details_account.data_len();

        OrderDetailStorage::with_mut_storage(order_details_account, |storage| {
            storage.insert_order(order_id, side, qty, price.clone(), current_account_size)?;
            msg!(
                "Placed order detail {} for user: side={:?}, qty={}, price={:?}",
                order_id,
                side,
                qty,
                price
            );
            Ok(())
        })
    }

    /// Place a pre-constructed OrderDetails in the user's storage
    /// This is a low-level storage operation that takes a constructed OrderDetails
    pub fn place_constructed_order_detail(
        order_details_account: &AccountInfo,
        order_details: &crate::state::order::OrderDetails,
    ) -> Result<(), ProgramError> {
        let current_account_size = order_details_account.data_len();

        OrderDetailStorage::with_mut_storage(order_details_account, |storage| {
            // Use the simpler insert_order method which creates its own OrderDetails
            storage.insert_order(
                order_details.order_id,
                order_details.side,
                order_details.qty,
                order_details.price.clone(),
                current_account_size,
            )?;

            msg!(
                "Stored order detail: id={}, side={:?}, qty={}, price={:?}",
                order_details.order_id,
                order_details.side,
                order_details.qty,
                order_details.price
            );

            Ok(())
        })
    }

    /// Cancel an order in the user's storage
    pub fn cancel_order_detail(
        order_details_account: &AccountInfo,
        order_id: u64,
    ) -> Result<(), ProgramError> {
        OrderDetailStorage::with_mut_storage(order_details_account, |storage| {
            storage.cancel_order(order_id)?;
            msg!("Cancelled order detail {}", order_id);
            Ok(())
        })
    }

    /// Fill an order (partially or completely)
    pub fn fill_order_detail(
        order_details_account: &AccountInfo,
        order_id: u64,
        fill_qty: u64,
    ) -> Result<(), ProgramError> {
        OrderDetailStorage::with_mut_storage(order_details_account, |storage| {
            storage.fill_order(order_id, fill_qty)?;
            msg!("Filled order detail {} with qty {}", order_id, fill_qty);
            Ok(())
        })
    }

    /// Update order tombstone to a specific state
    pub fn update_order_tombstone(
        order_details_account: &AccountInfo,
        order_id: u64,
        tombstone: OrderTombstone,
    ) -> Result<(), ProgramError> {
        OrderDetailStorage::with_mut_storage(order_details_account, |storage| {
            let tombstone_clone = tombstone.clone();
            storage.update_tombstone(order_id, tombstone)?;
            msg!(
                "Updated order detail {} tombstone to {:?}",
                order_id,
                tombstone_clone
            );
            Ok(())
        })
    }

    /// Get all active orders for the user
    pub fn get_active_orders(
        order_details_account: &AccountInfo,
    ) -> Result<Vec<u64>, ProgramError> {
        OrderDetailStorage::with_storage(order_details_account, |storage| {
            let active_orders = storage.get_active_orders();
            let order_ids: Vec<u64> = active_orders.into_iter().map(|(id, _)| id).collect();
            Ok(order_ids)
        })
    }

    /// Get storage statistics for monitoring
    pub fn get_storage_stats(
        order_details_account: &AccountInfo,
    ) -> Result<(usize, usize, usize, bool), ProgramError> {
        OrderDetailStorage::with_storage(order_details_account, |storage| {
            let stats = storage.stats();
            Ok((
                stats.capacity,
                stats.total_inserted,
                stats.active_orders,
                stats.is_ring_buffer,
            ))
        })
    }

    /// Get a specific order details by order_id
    pub fn get_order_details(
        order_details_account: &AccountInfo,
        order_id: u64,
    ) -> Result<(OrderSide, u64, u64, OrderPrice, OrderTombstone), ProgramError> {
        OrderDetailStorage::with_storage(order_details_account, |storage| {
            let order = storage.get_order(order_id)?;
            Ok((
                order.side,
                order.qty,
                order.filled_qty,
                order.price.clone(),
                order.tombstone.clone(),
            ))
        })
    }
}
