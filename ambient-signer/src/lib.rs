use borsh::{BorshDeserialize, BorshSerialize, to_vec};
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
};

/// Time in Force order types
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, PartialEq, Default)]
pub enum TimeInForce {
    /// Immediate or Cancel - execute immediately, cancel remainder
    IOC,
    /// Fill or Kill - execute completely or cancel entirely  
    FOK,
    /// Good Till Cancelled - remain active until cancelled
    #[default]
    GTC,
    /// Add Liquidity Only - only place if order rests
    ALO,
    /// Good Till Time - remain active until specified timestamp
    GTT(u64),
}

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedPermit {
    pub bytes: Vec<u8>,
    pub signature: [u8; 64],
}

impl SignedPermit {
    pub fn into_parts(self) -> (Vec<u8>, [u8; 64]) {
        (self.bytes, self.signature)
    }
}

/// Serialize and sign a permit envelope with the provided Ed25519 keypair.
pub fn sign_permit_ed25519(
    envelope: &PermitEnvelopeV1,
    keypair: &Keypair,
) -> Result<SignedPermit, std::io::Error> {
    let bytes = to_vec(&envelope)?;
    let signature = keypair.sign_message(&bytes).into();
    Ok(SignedPermit { bytes, signature })
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
    use solana_sdk::{pubkey, signer::SeedDerivable};
    use std::str::FromStr;

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

        let serialized = to_vec(&envelope).unwrap();
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

        assert!(to_vec(&floor1).is_ok());
        assert!(to_vec(&floor2).is_ok());
        assert!(to_vec(&floor3).is_ok());
    }

    #[test]
    fn test_sign_permit_ed25519() {
        let secret_key = [7u8; 32];
        let keypair = Keypair::from_seed(&secret_key).unwrap();

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
        assert_eq!(signed.bytes, to_vec(&envelope).unwrap());

        let expected: [u8; 64] = keypair.sign_message(&signed.bytes).into();
        assert_eq!(signed.signature, expected);
    }

}