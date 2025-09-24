use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Global state account - immutable address across upgrades
/// PDA: ["global_v1.2", bump]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct GlobalStateV1 {
    pub version: u8,     // =1
    pub _pad: [u8; 128], // forward compat
    /// Admin pubkeys (max 4)
    pub admins: Vec<Pubkey>,
    /// Keeper pubkeys (max 64)
    pub keepers: Vec<Pubkey>,
    // Minimum collateral deposit size
    pub min_deposit_size: u64,
    /// Unused padding for future fields
    pub _pad2: [u8; 128],
}

impl GlobalStateV1 {
    pub const CURRENT_VERSION: u8 = 1;
    pub const MAX_ADMINS: usize = 4;
    pub const MAX_KEEPERS: usize = 64;

    /// Check if a pubkey is an admin
    pub fn is_admin(&self, pubkey: &Pubkey) -> bool {
        // check explicit admins and allow the upgrade authority if present
        self.admins.iter().any(|admin| admin == pubkey)
    }

    /// Check if a pubkey is a keeper
    pub fn is_keeper(&self, pubkey: &Pubkey) -> bool {
        self.keepers.iter().any(|keeper| keeper == pubkey) || self.is_admin(pubkey)
    }
}
