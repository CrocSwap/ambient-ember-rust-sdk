use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

#[cfg(feature = "permit-signing")]
use ed25519_dalek::{Keypair as Ed25519Keypair, Signer as Ed25519Signer};

/// Domain separator for permit verification
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct PermitDomain {
    pub program_id: Pubkey,
    pub cluster: ClusterType,
    pub version: u8,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Copy, PartialEq)]
pub enum ClusterType {
    Mainnet,
    Testnet,
    Devnet,
    Localnet,
}

/// Replay prevention mode
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum ReplayMode {
    Sequence(u64),
    Nonce([u8; 32]),
    Allowance([u8; 32]),
    HlWindow { k: u8 },
}

/// Time in force for orders
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum TimeInForce {
    GTC,  // Good Till Cancel
    IOC,  // Immediate or Cancel
    FOK,  // Fill or Kill
}

/// Health floor specification
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub struct HealthFloor {
    pub metric: HealthMetric,
    pub min: i64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum HealthMetric {
    Initial,
    Maintenance,
    RatioBps,
}

/// Permit action variants
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum PermitAction {
    Place {
        market_id: u64,
        client_id: u128,
        side: u8,
        qty: u64,
        price: Option<u64>,
        tif: TimeInForce,
        reduce_only: bool,
        trigger_price: Option<u64>,
        trigger_type: u8,
        health_floor: Option<HealthFloor>,
    },
    CancelById {
        market_id: u64,
        order_id: u64,
    },
    CancelByClientId {
        market_id: u64,
        client_id: u128,
    },
    CancelAll {
        market_id: Option<u64>,
    },
    Modify {
        market_id: u64,
        cancel_order_id: u64,
        new_client_id: u128,
        side: u8,
        qty: u64,
        price: Option<u64>,
        tif: TimeInForce,
        reduce_only: bool,
        trigger_price: Option<u64>,
        trigger_type: u8,
        health_floor: Option<HealthFloor>,
    },
    Withdraw {
        amount: u64,
        to_owner: Pubkey,
        health_floor: Option<HealthFloor>,
    },
    SetLeverage {
        market_id: u64,
        target_leverage_bps: u16,
        health_floor: Option<HealthFloor>,
    },
    Noop,
    Faucet {
        market_id: u64,
        amount: u64,
        recipient: Pubkey,
    },
}

/// Main permit envelope structure (V1)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct PermitEnvelopeV1 {
    pub domain: PermitDomain,
    pub authorizer: Pubkey,
    pub key_type: KeyType,
    pub action: PermitAction,
    pub mode: ReplayMode,
    pub expires_unix: i64,
    pub max_fee_quote: u64,
    pub relayer: Option<Pubkey>,
    pub nonce: u64,
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq)]
pub enum KeyType {
    Ed25519,
    Secp256k1,
}

/// Result of signing a permit envelope with an Ed25519 keypair.
#[cfg(feature = "permit-signing")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedPermit {
    pub bytes: Vec<u8>,
    pub signature: [u8; 64],
}

#[cfg(feature = "permit-signing")]
impl SignedPermit {
    pub fn into_parts(self) -> (Vec<u8>, [u8; 64]) {
        (self.bytes, self.signature)
    }
}

/// Serialize and sign a permit envelope with the provided Ed25519 keypair.
#[cfg(feature = "permit-signing")]
pub fn sign_permit_ed25519(
    envelope: &PermitEnvelopeV1,
    keypair: &Ed25519Keypair,
) -> Result<SignedPermit, std::io::Error> {
    let bytes = envelope.try_to_vec()?;
    let signature = keypair.sign(&bytes).to_bytes();
    Ok(SignedPermit { bytes, signature })
}

/// Generate a new Ed25519 keypair for testing
#[cfg(feature = "permit-signing")]
pub fn generate_keypair() -> Ed25519Keypair {
    use rand::rngs::OsRng;
    
    Ed25519Keypair::generate(&mut OsRng)
}

/// Convert hex string to bytes
pub fn hex_to_bytes(hex_str: &str) -> Result<Vec<u8>, hex::FromHexError> {
    hex::decode(hex_str)
}

/// Convert bytes to hex string
pub fn bytes_to_hex(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn test_permit_envelope_serialization() {
        let envelope = PermitEnvelopeV1 {
            domain: PermitDomain {
                program_id: Pubkey::new_unique(),
                cluster: ClusterType::Testnet,
                version: 1,
            },
            authorizer: Pubkey::new_unique(),
            key_type: KeyType::Ed25519,
            action: PermitAction::Place {
                market_id: 1,
                client_id: 12345,
                side: 0,
                qty: 100,
                price: Some(50000),
                tif: TimeInForce::GTC,
                reduce_only: false,
                trigger_price: None,
                trigger_type: 0,
                health_floor: None,
            },
            mode: ReplayMode::HlWindow { k: 128 },
            expires_unix: 1700000000,
            max_fee_quote: 0,
            relayer: None,
            nonce: 123456789,
        };

        let serialized = envelope.try_to_vec().unwrap();
        let deserialized = PermitEnvelopeV1::try_from_slice(&serialized).unwrap();

        assert_eq!(envelope.domain.version, deserialized.domain.version);
        assert_eq!(envelope.authorizer, deserialized.authorizer);
        assert_eq!(envelope.nonce, deserialized.nonce);
        assert_eq!(envelope.expires_unix, deserialized.expires_unix);
    }

    #[test]
    fn test_health_floor_variants() {
        let floor1 = HealthFloor {
            metric: HealthMetric::Initial,
            min: 0,
        };

        let floor2 = HealthFloor {
            metric: HealthMetric::Maintenance,
            min: 1000,
        };

        let floor3 = HealthFloor {
            metric: HealthMetric::RatioBps,
            min: 150,
        };

        assert!(floor1.try_to_vec().is_ok());
        assert!(floor2.try_to_vec().is_ok());
        assert!(floor3.try_to_vec().is_ok());
    }

    #[cfg(feature = "permit-signing")]
    #[test]
    fn test_sign_permit_ed25519() {
        let keypair = generate_keypair();

        let envelope = PermitEnvelopeV1 {
            domain: PermitDomain {
                program_id: Pubkey::new_unique(),
                cluster: ClusterType::Testnet,
                version: 1,
            },
            authorizer: Pubkey::new_unique(),
            key_type: KeyType::Ed25519,
            action: PermitAction::Noop,
            mode: ReplayMode::HlWindow { k: 16 },
            expires_unix: 1_700_000_000,
            max_fee_quote: 0,
            relayer: None,
            nonce: 42,
        };

        let signed = sign_permit_ed25519(&envelope, &keypair).expect("signing should succeed");
        assert_eq!(signed.bytes, envelope.try_to_vec().unwrap());

        let expected = keypair.sign(&signed.bytes).to_bytes();
        assert_eq!(signed.signature, expected);
    }

    #[test]
    fn test_hex_conversions() {
        let test_bytes = vec![0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        let hex_string = bytes_to_hex(&test_bytes);
        assert_eq!(hex_string, "0123456789abcdef");

        let converted_back = hex_to_bytes(&hex_string).unwrap();
        assert_eq!(converted_back, test_bytes);
    }

    #[cfg(feature = "permit-signing")]
    #[test]
    fn test_generate_keypair() {
        let keypair1 = generate_keypair();
        let keypair2 = generate_keypair();
        
        // Two generated keypairs should be different
        assert_ne!(keypair1.secret.to_bytes(), keypair2.secret.to_bytes());
    }
}