/// Ember on-chain error codes, matching the design spec.
/// These are returned as u32 discriminants in program errors.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmberError {
    /// 0: Invalid ix discriminant
    UnknownInstruction = 0,
    /// 1: Seeds mismatch or account not owned by program
    InvalidPDA = 1,
    /// 2: Signer lacks required role
    Unauthorized = 2,
    /// 3: Account version > program CURR_VERSION
    VersionMismatch = 3,
    /// 4: Not enough free collateral
    InsufficientNonCommitted = 4,
    /// 5: committed_collateral too low
    InsufficientCommitted = 5,
    /// 6: OrderRegistry at capacity
    RegistryFull = 6,
    /// 7: order_id missing
    OrderNotFound = 7,
    /// 8: Re-used order_id
    DuplicateOrderID = 8,
    /// 9: Would breach init margin
    WithdrawalBelowIM = 9,
    /// 10: Keeper attempted premature liq
    LiquidationNotEligible = 10,
    /// 11: Borsh (de)ser failure
    SerializationError = 11,
    /// 12: u64 overflow/underflow
    MathOverflow = 12,
}

impl From<EmberError> for u32 {
    fn from(e: EmberError) -> Self {
        e as u32
    }
}

impl std::fmt::Display for EmberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use EmberError::*;
        let msg = match self {
            UnknownInstruction => "UnknownInstruction: Invalid ix discriminant",
            InvalidPDA => "InvalidPDA: Seeds mismatch or account not owned by program",
            Unauthorized => "Unauthorized: Signer lacks required role",
            VersionMismatch => "VersionMismatch: Account version > program CURR_VERSION",
            InsufficientNonCommitted => "InsufficientNonCommitted: Not enough free collateral",
            InsufficientCommitted => "InsufficientCommitted: committed_collateral too low",
            RegistryFull => "RegistryFull: OrderRegistry at capacity",
            OrderNotFound => "OrderNotFound: order_id missing",
            DuplicateOrderID => "DuplicateOrderID: Re-used order_id",
            WithdrawalBelowIM => "WithdrawalBelowIM: Would breach init margin",
            LiquidationNotEligible => "LiquidationNotEligible: Keeper attempted premature liq",
            SerializationError => "SerializationError: Borsh (de)ser failure",
            MathOverflow => "MathOverflow: u64 overflow/underflow",
        };
        write!(f, "{}", msg)
    }
}
