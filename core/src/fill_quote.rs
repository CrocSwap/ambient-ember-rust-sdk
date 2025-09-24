use crate::OrderSide;
use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Data structure that a trusted keeper signs to authorize a fill
/// This allows anyone to submit the fill transaction while maintaining security
#[repr(C)]
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct OffchainFillQuote {
    /// Order ID to fill (matches the user's order_id)
    pub order_id: u64,
    /// Side of the order (must match the original order)
    pub side: OrderSide,
    /// Quantity to fill in base units (must not exceed remaining quantity)
    pub fill_qty: u64,
    /// Fill price in fixed-point format (same as price in orders)
    pub fill_price: u64,
    /// Last slot at which this quote is valid (prevents replay attacks)
    pub expiry_slot: u64,
    /// Market ID for this fill (prevents cross-market replay)
    pub market_id: u64,
    /// User who owns the order (prevents user spoofing)
    pub user: Pubkey,
    /// Nonce to prevent replay attacks (keeper-maintained, monotonic)
    pub nonce: u64,
}

impl OffchainFillQuote {
    /// Create a new fill quote with validation
    pub fn new(
        order_id: u64,
        side: OrderSide,
        fill_qty: u64,
        fill_price: u64,
        expiry_slot: u64,
        market_id: u64,
        user: Pubkey,
        nonce: u64,
    ) -> Result<Self, FillQuoteError> {
        if fill_qty == 0 {
            return Err(FillQuoteError::InvalidQuantity);
        }

        if fill_price == 0 {
            return Err(FillQuoteError::InvalidPrice);
        }

        Ok(Self {
            order_id,
            side,
            fill_qty,
            fill_price,
            expiry_slot,
            market_id,
            user,
            nonce,
        })
    }

    /// Serialize the quote to bytes for hashing and signing
    pub fn to_bytes(&self) -> Result<Vec<u8>, FillQuoteError> {
        self.try_to_vec()
            .map_err(|_| FillQuoteError::SerializationError)
    }

    /// Calculate the message hash that should be signed
    pub fn message_hash(&self) -> Result<[u8; 32], FillQuoteError> {
        use solana_program::hash::hash;
        let bytes = self.to_bytes()?;
        Ok(hash(&bytes).to_bytes())
    }
}

/// Complete fill quote payload including signature verification data
#[derive(Debug, Clone)]
pub struct SignedFillQuote {
    /// The quote data that was signed
    pub quote: OffchainFillQuote,
    /// The ed25519 signature (64 bytes)
    pub signature: [u8; 64],
    /// The keeper's public key that signed this quote
    pub keeper_pubkey: Pubkey,
}

impl SignedFillQuote {
    pub fn new(quote: OffchainFillQuote, signature: [u8; 64], keeper_pubkey: Pubkey) -> Self {
        Self {
            quote,
            signature,
            keeper_pubkey,
        }
    }

    /// Get the raw quote bytes for verification
    pub fn quote_bytes(&self) -> Result<Vec<u8>, FillQuoteError> {
        self.quote.to_bytes()
    }

    /// Get the message hash that should have been signed
    pub fn message_hash(&self) -> Result<[u8; 32], FillQuoteError> {
        self.quote.message_hash()
    }
}

/// Errors that can occur when working with fill quotes
#[derive(Debug, PartialEq)]
pub enum FillQuoteError {
    InvalidQuantity,
    InvalidPrice,
    SerializationError,
    SignatureVerificationFailed,
    QuoteExpired,
    KeeperNotAuthorized,
    InvalidNonce,
    OrderNotFound,
    OrderAlreadyFilled,
    OrderCanceled,
    SideMismatch,
    UserMismatch,
    MarketMismatch,
    InsufficientRemainingQuantity,
}

impl std::fmt::Display for FillQuoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FillQuoteError::InvalidQuantity => write!(f, "Invalid fill quantity"),
            FillQuoteError::InvalidPrice => write!(f, "Invalid fill price"),
            FillQuoteError::SerializationError => write!(f, "Failed to serialize quote"),
            FillQuoteError::SignatureVerificationFailed => {
                write!(f, "Signature verification failed")
            }
            FillQuoteError::QuoteExpired => write!(f, "Quote has expired"),
            FillQuoteError::KeeperNotAuthorized => write!(f, "Keeper not authorized"),
            FillQuoteError::InvalidNonce => write!(f, "Invalid nonce"),
            FillQuoteError::OrderNotFound => write!(f, "Order not found"),
            FillQuoteError::OrderAlreadyFilled => write!(f, "Order already filled"),
            FillQuoteError::OrderCanceled => write!(f, "Order is canceled"),
            FillQuoteError::SideMismatch => write!(f, "Order side mismatch"),
            FillQuoteError::UserMismatch => write!(f, "User mismatch"),
            FillQuoteError::MarketMismatch => write!(f, "Market mismatch"),
            FillQuoteError::InsufficientRemainingQuantity => {
                write!(f, "Insufficient remaining quantity")
            }
        }
    }
}

impl std::error::Error for FillQuoteError {}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_program::pubkey::Pubkey;

    #[test]
    fn test_fill_quote_creation() {
        let user = Pubkey::new_unique();
        let quote = OffchainFillQuote::new(
            123,            // order_id
            OrderSide::Bid, // side
            1000000,        // fill_qty
            50000,          // fill_price
            1000,           // expiry_slot
            42,             // market_id
            user,           // user
            1,              // nonce
        )
        .unwrap();

        assert_eq!(quote.order_id, 123);
        assert_eq!(quote.side, OrderSide::Bid);
        assert_eq!(quote.fill_qty, 1000000);
        assert_eq!(quote.fill_price, 50000);
        assert_eq!(quote.expiry_slot, 1000);
        assert_eq!(quote.market_id, 42);
        assert_eq!(quote.user, user);
        assert_eq!(quote.nonce, 1);
    }

    #[test]
    fn test_fill_quote_validation() {
        let user = Pubkey::new_unique();

        // Invalid quantity
        let result = OffchainFillQuote::new(123, OrderSide::Bid, 0, 50000, 1000, 42, user, 1);
        assert_eq!(result.unwrap_err(), FillQuoteError::InvalidQuantity);

        // Invalid price
        let result = OffchainFillQuote::new(123, OrderSide::Bid, 1000000, 0, 1000, 42, user, 1);
        assert_eq!(result.unwrap_err(), FillQuoteError::InvalidPrice);
    }

    #[test]
    fn test_fill_quote_serialization() {
        let user = Pubkey::new_unique();
        let quote =
            OffchainFillQuote::new(123, OrderSide::Bid, 1000000, 50000, 1000, 42, user, 1).unwrap();

        let bytes = quote.to_bytes().unwrap();
        assert!(!bytes.is_empty());

        // Should be able to deserialize back
        let deserialized = OffchainFillQuote::try_from_slice(&bytes).unwrap();
        assert_eq!(deserialized, quote);
    }

    #[test]
    fn test_message_hash_deterministic() {
        let user = Pubkey::new_unique();
        let quote =
            OffchainFillQuote::new(123, OrderSide::Bid, 1000000, 50000, 1000, 42, user, 1).unwrap();

        let hash1 = quote.message_hash().unwrap();
        let hash2 = quote.message_hash().unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_signed_fill_quote() {
        let user = Pubkey::new_unique();
        let keeper = Pubkey::new_unique();
        let quote =
            OffchainFillQuote::new(123, OrderSide::Bid, 1000000, 50000, 1000, 42, user, 1).unwrap();

        let signature = [0u8; 64];
        let signed_quote = SignedFillQuote::new(quote.clone(), signature, keeper);

        assert_eq!(signed_quote.quote, quote);
        assert_eq!(signed_quote.signature, signature);
        assert_eq!(signed_quote.keeper_pubkey, keeper);
    }
}
