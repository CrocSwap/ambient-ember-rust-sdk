use crate::state::order::OrderDetails;
use crate::storage::market_order_log::{FillLogDetails, MarketOrderLogStats, OrderUpdateType};
use crate::storage::zero_copy_market_order_log::{ZeroCopyMarketOrderLog, ZeroCopyOrderLogInfo};
use solana_program::{account_info::AccountInfo, program_error::ProgramError, pubkey::Pubkey};

/// High-level wrapper around ZeroCopyMarketOrderLog that provides
/// the same API as the original MarketOrderLog but with O(1) append operations
pub struct MarketOrderLogWrapper<'a> {
    inner: ZeroCopyMarketOrderLog<'a>,
}

impl<'a> MarketOrderLogWrapper<'a> {
    /// Create a new empty market order log (initializes the account)
    pub fn new(
        account: &AccountInfo<'a>,
        market_id: u64,
        page: u32,
        capacity: u64,
    ) -> Result<Self, ProgramError> {
        ZeroCopyMarketOrderLog::init_in_account(account, market_id, page, capacity)?;
        let inner = ZeroCopyMarketOrderLog::load(account)?;
        Ok(Self { inner })
    }

    /// Load an existing market order log from account data
    pub fn load(account: &AccountInfo<'a>) -> Result<Self, ProgramError> {
        let inner = ZeroCopyMarketOrderLog::load(account)?;
        Ok(Self { inner })
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
        self.inner.append_entry(
            user,
            order_id,
            update_type,
            order_details,
            fill_details,
            slot,
        )
    }

    /// Append a user collateral update to the log
    pub fn append_user_collateral_update(
        &mut self,
        user: Pubkey,
        collateral_snapshot: u64,
        slot: u64,
    ) -> Result<(), ProgramError> {
        self.inner
            .append_user_collateral_update(user, collateral_snapshot, slot)
    }

    /// Check if the log needs reallocation
    pub fn needs_realloc(&self, current_account_size: usize) -> bool {
        self.inner.needs_realloc(current_account_size)
    }

    /// Estimate the serialized size of the log
    pub fn estimated_serialized_size(&self) -> usize {
        self.inner.estimated_serialized_size()
    }

    /// Get statistics about the log
    pub fn get_stats(&self) -> Result<MarketOrderLogStats, ProgramError> {
        self.inner.get_stats()
    }

    /// Get basic information about the log
    pub fn get_info(&self) -> Result<ZeroCopyOrderLogInfo, ProgramError> {
        self.inner.get_basic_info()
    }

    /// Get market ID
    pub fn market_id(&self) -> Result<u64, ProgramError> {
        Ok(self.inner.get_basic_info()?.market_id)
    }

    /// Get page number
    pub fn page(&self) -> Result<u32, ProgramError> {
        Ok(self.inner.get_basic_info()?.page)
    }

    /// Get entry count
    pub fn entry_count(&self) -> Result<u64, ProgramError> {
        Ok(self.inner.get_basic_info()?.entry_count)
    }

    /// Get capacity
    pub fn capacity(&self) -> Result<u64, ProgramError> {
        Ok(self.inner.get_basic_info()?.capacity)
    }

    /// Check if the log is full
    pub fn is_full(&self) -> Result<bool, ProgramError> {
        let info = self.inner.get_basic_info()?;
        Ok(info.entry_count >= info.capacity)
    }

    /// Get the utilization percentage
    pub fn utilization_percent(&self) -> Result<u64, ProgramError> {
        let info = self.inner.get_basic_info()?;
        if info.capacity > 0 {
            Ok((info.entry_count * 100) / info.capacity)
        } else {
            Ok(0)
        }
    }

    /// Get the entry size in bytes
    pub fn entry_size(&self) -> usize {
        crate::storage::zero_copy_market_order_log::get_entry_serialized_size().unwrap_or(0)
    }

    /// Update the capacity (typically after reallocation)
    pub fn update_capacity(&mut self, new_capacity: u64) -> Result<(), ProgramError> {
        self.inner.update_capacity(new_capacity)
    }

    /// Get access to the underlying zero-copy implementation for advanced operations
    pub fn inner(&self) -> &ZeroCopyMarketOrderLog<'a> {
        &self.inner
    }

    /// Get mutable access to the underlying zero-copy implementation for advanced operations
    pub fn inner_mut(&mut self) -> &mut ZeroCopyMarketOrderLog<'a> {
        &mut self.inner
    }

    /// Iterate over all entries (convenience method)
    pub fn iter_entries(
        &self,
    ) -> impl Iterator<
        Item = Result<
            crate::storage::market_order_log::OrderLogEntry,
            crate::storage::zero_copy_market_order_log::ZeroCopyOrderLogError,
        >,
    > + '_ {
        self.inner.iter_entries()
    }

    /// Iterate over entries in a range (convenience method)
    pub fn iter_entries_range(
        &self,
        start: u64,
        end: u64,
    ) -> impl Iterator<
        Item = Result<
            crate::storage::market_order_log::OrderLogEntry,
            crate::storage::zero_copy_market_order_log::ZeroCopyOrderLogError,
        >,
    > + '_ {
        self.inner.iter_entries_range(start, end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn test_wrapper_basic_operations() {
        // Calculate required size for a log with capacity 10
        let entry_size =
            crate::storage::zero_copy_market_order_log::get_entry_serialized_size().unwrap();
        let header_size =
            std::mem::size_of::<crate::storage::zero_copy_market_order_log::MarketOrderLogHeader>();
        let capacity = 10u64;
        let required_size = header_size + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        // Create new wrapper
        let mut wrapper = MarketOrderLogWrapper::new(&account, 42, 0, capacity).unwrap();

        // Test basic info
        assert_eq!(wrapper.market_id().unwrap(), 42);
        assert_eq!(wrapper.page().unwrap(), 0);
        assert_eq!(wrapper.entry_count().unwrap(), 0);
        assert_eq!(wrapper.capacity().unwrap(), capacity);
        assert!(!wrapper.is_full().unwrap());
        assert_eq!(wrapper.utilization_percent().unwrap(), 0);

        // Create test order details
        let user = Pubkey::new_unique();
        let order_details = create_representative_order_details();

        // Append entry
        wrapper
            .append_entry(
                user,
                12345,
                OrderUpdateType::OrderEntry,
                order_details,
                None,
                100000,
            )
            .unwrap();

        // Check updated info
        assert_eq!(wrapper.entry_count().unwrap(), 1);
        assert_eq!(wrapper.utilization_percent().unwrap(), 10); // 1/10 * 100 = 10%

        // Get stats
        let stats = wrapper.get_stats().unwrap();
        assert_eq!(stats.total_entries, 1);
        assert_eq!(stats.order_entries, 1);
    }

    #[test]
    fn test_wrapper_load_existing() {
        let entry_size =
            crate::storage::zero_copy_market_order_log::get_entry_serialized_size().unwrap();
        let header_size =
            std::mem::size_of::<crate::storage::zero_copy_market_order_log::MarketOrderLogHeader>();
        let capacity = 5u64;
        let required_size = header_size + (capacity as usize * entry_size);

        let (mut lamports, mut data) = create_test_account_data(required_size);
        let account = make_account_info(&mut lamports, &mut data);

        // First, create and populate a log
        {
            let mut wrapper = MarketOrderLogWrapper::new(&account, 99, 1, capacity).unwrap();
            let user = Pubkey::new_unique();
            let order_details = create_representative_order_details();

            wrapper
                .append_entry(
                    user,
                    1,
                    OrderUpdateType::OrderEntry,
                    order_details.clone(),
                    None,
                    1000,
                )
                .unwrap();
            wrapper
                .append_entry(user, 2, OrderUpdateType::Fill, order_details, None, 1001)
                .unwrap();
        }

        // Then, load the existing log
        let wrapper = MarketOrderLogWrapper::load(&account).unwrap();
        assert_eq!(wrapper.market_id().unwrap(), 99);
        assert_eq!(wrapper.page().unwrap(), 1);
        assert_eq!(wrapper.entry_count().unwrap(), 2);
        assert_eq!(wrapper.capacity().unwrap(), capacity);

        let stats = wrapper.get_stats().unwrap();
        assert_eq!(stats.total_entries, 2);
        assert_eq!(stats.order_entries, 1);
        assert_eq!(stats.fills, 1);
    }
}
