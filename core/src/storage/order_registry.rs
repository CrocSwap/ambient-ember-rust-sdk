use crate::state::order::OrderMarker;
use crate::storage::order_storage::{OrderStorage, OrderStorageError};
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{account_info::AccountInfo, msg, program_error::ProgramError};

/// Helper functions for working with OrderStorage in the OrderRegistry PDA
impl OrderStorage {
    /// Initialize a new OrderRegistry PDA with OrderStorage
    /// Should be called when creating the PDA for the first time
    pub fn init_in_account(
        order_registry_account: &AccountInfo,
        capacity: usize,
    ) -> Result<(), ProgramError> {
        if !order_registry_account.data_is_empty() {
            msg!("Error: OrderRegistry already initialized");
            return Err(ProgramError::AccountAlreadyInitialized);
        }

        let storage = OrderStorage::new(capacity);
        let serialized = storage.try_to_vec().map_err(|_| {
            ProgramError::BorshIoError("Failed to serialize OrderStorage".to_string())
        })?;

        // Check if account is large enough
        if order_registry_account.data_len() < serialized.len() {
            msg!(
                "Error: OrderRegistry account too small. Need {} bytes, have {}",
                serialized.len(),
                order_registry_account.data_len()
            );
            return Err(ProgramError::AccountDataTooSmall);
        }

        let mut data = order_registry_account.try_borrow_mut_data()?;
        data[..serialized.len()].copy_from_slice(&serialized);

        msg!("OrderRegistry initialized with capacity: {}", capacity);
        Ok(())
    }

    /// Load OrderStorage from OrderRegistry PDA
    pub fn load_from_account(order_registry_account: &AccountInfo) -> Result<Self, ProgramError> {
        if order_registry_account.data_is_empty() {
            msg!("Error: OrderRegistry not initialized");
            return Err(ProgramError::UninitializedAccount);
        }

        let data = order_registry_account.try_borrow_data()?;
        OrderStorage::try_from_slice(&data).map_err(|e| {
            msg!("Failed to deserialize OrderStorage: {}", e);
            ProgramError::BorshIoError("Failed to deserialize OrderStorage".to_string())
        })
    }

    /// Save OrderStorage back to OrderRegistry PDA
    pub fn save_to_account(
        &self,
        order_registry_account: &AccountInfo,
    ) -> Result<(), ProgramError> {
        let serialized = self.try_to_vec().map_err(|_| {
            ProgramError::BorshIoError("Failed to serialize OrderStorage".to_string())
        })?;

        if order_registry_account.data_len() < serialized.len() {
            msg!("Error: OrderRegistry account too small for updated data");
            return Err(ProgramError::AccountDataTooSmall);
        }

        let mut data = order_registry_account.try_borrow_mut_data()?;
        data[..serialized.len()].copy_from_slice(&serialized);

        Ok(())
    }

    /// Atomic operation: Load -> Modify -> Save
    /// Use this for operations that need to modify the storage
    pub fn with_mut_storage<F, R>(
        order_registry_account: &AccountInfo,
        f: F,
    ) -> Result<R, ProgramError>
    where
        F: FnOnce(&mut OrderStorage) -> Result<R, OrderStorageError>,
    {
        let mut storage = Self::load_from_account(order_registry_account)?;

        let result = f(&mut storage).map_err(|e| {
            msg!("OrderStorage operation failed: {:?}", e);
            match e {
                OrderStorageError::StorageFull => ProgramError::Custom(100),
                OrderStorageError::OrderNotFound => ProgramError::Custom(101),
                OrderStorageError::DuplicateOrder => ProgramError::Custom(102),
                OrderStorageError::InvalidCapacity => ProgramError::Custom(103),
                OrderStorageError::InvalidIndex => ProgramError::Custom(104),
            }
        })?;

        storage.save_to_account(order_registry_account)?;
        Ok(result)
    }

    /// Read-only operation: Load -> Read
    /// Use this for operations that only need to read the storage
    pub fn with_storage<F, R>(order_registry_account: &AccountInfo, f: F) -> Result<R, ProgramError>
    where
        F: FnOnce(&OrderStorage) -> Result<R, OrderStorageError>,
    {
        let storage = Self::load_from_account(order_registry_account)?;

        f(&storage).map_err(|e| {
            msg!("OrderStorage operation failed: {:?}", e);
            match e {
                OrderStorageError::StorageFull => ProgramError::Custom(100),
                OrderStorageError::OrderNotFound => ProgramError::Custom(101),
                OrderStorageError::DuplicateOrder => ProgramError::Custom(102),
                OrderStorageError::InvalidCapacity => ProgramError::Custom(103),
                OrderStorageError::InvalidIndex => ProgramError::Custom(104),
            }
        })
    }
}

/// Convenience functions for common order operations
pub mod order_ops {
    use super::*;
    use solana_program::pubkey::Pubkey;

    /// Insert an order into the OrderRegistry
    pub fn insert_order(
        order_registry_account: &AccountInfo,
        order: OrderMarker,
    ) -> Result<usize, ProgramError> {
        OrderStorage::with_mut_storage(order_registry_account, |storage| {
            let index = storage.insert(order)?;
            msg!(
                "Inserted order at slot {}, total orders: {}",
                index,
                storage.count
            );
            Ok(index)
        })
    }

    /// Remove an order from the OrderRegistry
    pub fn remove_order(
        order_registry_account: &AccountInfo,
        owner: &Pubkey,
        order_id: u64,
        hint: Option<usize>,
    ) -> Result<OrderMarker, ProgramError> {
        OrderStorage::with_mut_storage(order_registry_account, |storage| {
            let order = storage.remove(owner, order_id, hint)?;
            msg!(
                "Removed order {}, total orders: {}",
                order_id,
                storage.count
            );
            Ok(order)
        })
    }

    /// Find an order in the OrderRegistry
    pub fn find_order(
        order_registry_account: &AccountInfo,
        owner: &Pubkey,
        order_id: u64,
        hint: Option<usize>,
    ) -> Result<Option<(usize, OrderMarker)>, ProgramError> {
        OrderStorage::with_storage(order_registry_account, |storage| {
            if let Some(index) = storage.find_by_owner_and_id(owner, order_id, hint) {
                let order = storage.get(index)?;
                Ok(Some((index, order.clone())))
            } else {
                Ok(None)
            }
        })
    }

    /// Get storage statistics
    pub fn get_stats(
        order_registry_account: &AccountInfo,
    ) -> Result<(usize, usize, usize), ProgramError> {
        OrderStorage::with_storage(order_registry_account, |storage| {
            let stats = storage.stats();
            Ok((stats.capacity, stats.count, stats.utilization_pct))
        })
    }

    /// Resize the OrderRegistry storage
    pub fn resize_storage(
        order_registry_account: &AccountInfo,
        new_capacity: usize,
    ) -> Result<(), ProgramError> {
        OrderStorage::with_mut_storage(order_registry_account, |storage| {
            storage.resize(new_capacity)?;
            msg!("Resized OrderRegistry to capacity: {}", new_capacity);
            Ok(())
        })
    }

    /// Get all orders for a specific user
    pub fn get_user_orders(
        order_registry_account: &AccountInfo,
        user: &Pubkey,
    ) -> Result<Vec<(usize, OrderMarker)>, ProgramError> {
        OrderStorage::with_storage(order_registry_account, |storage| {
            let orders: Vec<(usize, OrderMarker)> = storage
                .iter_user_orders(user)
                .map(|(index, order)| (index, order.clone()))
                .collect();
            Ok(orders)
        })
    }
}
