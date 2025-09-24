use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

#[cfg(feature = "permit-signing")]
use ed25519_dalek::{Keypair as Ed25519Keypair, Signer as Ed25519Signer};

use crate::TimeInForce;

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

/// Stored sequence replay state
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Default)]
pub struct SequenceState {
    pub next_sequence: u64,
    pub bump: u8,
}

impl SequenceState {
    pub const LEN: usize = 8 + 1;
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

/// Session PDA state
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct SessionState {
    pub owner: Pubkey,
    pub session: Pubkey,
    pub expires_unix: i64,
    pub scopes_bits: u32,
    pub withdraw_limit_24h: u64,
    pub per_market_size_limit_lots: i64,
    pub bump: u8,
}

impl SessionState {
    pub const LEN: usize = 32 + 32 + 8 + 4 + 8 + 8 + 1;

    pub fn has_scope(&self, action: &PermitAction) -> bool {
        match action {
            PermitAction::Place { .. } => (self.scopes_bits & SCOPE_PLACE) != 0,
            PermitAction::CancelById { .. }
            | PermitAction::CancelByClientId { .. }
            | PermitAction::CancelAll { .. } => (self.scopes_bits & SCOPE_CANCEL) != 0,
            PermitAction::Modify { .. } => {
                (self.scopes_bits & SCOPE_CANCEL) != 0 && (self.scopes_bits & SCOPE_PLACE) != 0
            }
            PermitAction::Withdraw { .. } => (self.scopes_bits & SCOPE_WITHDRAW) != 0,
            PermitAction::SetLeverage { .. } => (self.scopes_bits & SCOPE_SET_LEVERAGE) != 0,
            PermitAction::Faucet { .. } => (self.scopes_bits & SCOPE_FAUCET) != 0,
            PermitAction::Noop => true,
        }
    }
}

pub const SCOPE_PLACE: u32 = 1 << 0;
pub const SCOPE_CANCEL: u32 = 1 << 1;
pub const SCOPE_WITHDRAW: u32 = 1 << 2;
pub const SCOPE_SET_LEVERAGE: u32 = 1 << 3;
pub const SCOPE_FAUCET: u32 = 1 << 4;

/// Allowance PDA state
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct AllowanceState {
    pub owner: Pubkey,
    pub session: Pubkey,
    pub id: [u8; 32],
    pub remaining_uses: u32,
    pub expires_unix: i64,
    pub scopes_bits: u32,
    pub bump: u8,
}

impl AllowanceState {
    pub const LEN: usize = 32 + 32 + 32 + 4 + 8 + 4 + 1;
}

/// Used nonce PDA state (minimal marker)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct UsedNonceState {
    pub timestamp: i64,
    pub bump: u8,
}

impl UsedNonceState {
    pub const LEN: usize = 8 + 1;
}

/// Nonce window PDA state (Hyperliquid-style window)
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct NonceWindowState {
    pub signer: Pubkey,
    pub k: u8,
    pub top: Vec<u64>,
    pub bump: u8,
}

impl NonceWindowState {
    pub const MAX_K: u8 = 128;
    pub const LEN: usize = 32 + 1 + 4 + ((Self::MAX_K as usize) * 8) + 1; // Max size with k=128

    pub fn insert_nonce(&mut self, nonce: u64) -> Result<(), &'static str> {
        if self.top.contains(&nonce) {
            return Err("Nonce already used");
        }

        if self.top.len() < self.k as usize {
            self.top.push(nonce);
            self.top.sort_unstable();
            return Ok(());
        }

        let smallest = self.top[0];
        if nonce <= smallest {
            return Err("Nonce too small for window");
        }

        self.top[0] = nonce;
        self.top.sort_unstable();
        Ok(())
    }

    pub fn is_valid_nonce(&self, nonce: u64, now_ms: i64) -> bool {
        let two_days_ms = 2 * 24 * 60 * 60 * 1000;
        let one_day_ms = 24 * 60 * 60 * 1000;

        let nonce_time = nonce as i64;
        if nonce_time < (now_ms - two_days_ms) || nonce_time > (now_ms + one_day_ms) {
            return false;
        }

        true
    }
}

/// Event emitted when a permit is consumed
#[derive(BorshSerialize, BorshDeserialize, Debug)]
pub struct PermitConsumedEvent {
    pub owner: Pubkey,
    pub authorizer: Pubkey,
    pub action_hash: [u8; 32],
    pub nonce: u64,
    pub timestamp: i64,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "permit-signing")]
    use ed25519_dalek::{PublicKey, SecretKey};

    #[test]
    fn test_nonce_window_insert() {
        let mut window = NonceWindowState {
            signer: Pubkey::new_unique(),
            k: 3,
            top: vec![],
            bump: 0,
        };

        assert!(window.insert_nonce(100).is_ok());
        assert_eq!(window.top, vec![100]);

        assert!(window.insert_nonce(200).is_ok());
        assert_eq!(window.top, vec![100, 200]);

        assert!(window.insert_nonce(150).is_ok());
        assert_eq!(window.top, vec![100, 150, 200]);

        assert!(window.insert_nonce(300).is_ok());
        assert_eq!(window.top, vec![150, 200, 300]);

        assert!(window.insert_nonce(140).is_err());
        assert_eq!(window.top, vec![150, 200, 300]);

        assert!(window.insert_nonce(200).is_err());
        assert_eq!(window.top, vec![150, 200, 300]);
    }

    #[test]
    fn test_nonce_window_time_bounds() {
        let window = NonceWindowState {
            signer: Pubkey::new_unique(),
            k: 128,
            top: vec![],
            bump: 0,
        };

        let now_ms: i64 = 1_700_000_000_000;
        let one_day_ms = 24 * 60 * 60 * 1000;
        let two_days_ms = 2 * one_day_ms;

        assert!(window.is_valid_nonce(now_ms as u64, now_ms));
        assert!(window.is_valid_nonce((now_ms - one_day_ms) as u64, now_ms));
        assert!(window.is_valid_nonce((now_ms + (one_day_ms / 2)) as u64, now_ms));

        assert!(!window.is_valid_nonce((now_ms - two_days_ms - 1000) as u64, now_ms));
        assert!(!window.is_valid_nonce((now_ms + one_day_ms + 1000) as u64, now_ms));
    }

    #[test]
    fn test_session_has_scope() {
        let session = SessionState {
            owner: Pubkey::new_unique(),
            session: Pubkey::new_unique(),
            expires_unix: 0,
            scopes_bits: SCOPE_PLACE | SCOPE_CANCEL,
            withdraw_limit_24h: 0,
            per_market_size_limit_lots: 0,
            bump: 0,
        };

        assert!(session.has_scope(&PermitAction::Place {
            market_id: 1,
            client_id: 123,
            side: 0,
            qty: 100,
            price: Some(1000),
            tif: TimeInForce::GTC,
            reduce_only: false,
            trigger_price: None,
            trigger_type: 0,
            health_floor: None,
        }));

        assert!(session.has_scope(&PermitAction::CancelById {
            market_id: 1,
            order_id: 123,
        }));

        assert!(!session.has_scope(&PermitAction::Withdraw {
            amount: 1000,
            to_owner: Pubkey::new_unique(),
            health_floor: None,
        }));

        assert!(!session.has_scope(&PermitAction::SetLeverage {
            market_id: 1,
            target_leverage_bps: 200,
            health_floor: None,
        }));

        assert!(!session.has_scope(&PermitAction::Faucet {
            market_id: 1,
            amount: 1_000,
            recipient: Pubkey::new_unique(),
        }));

        let faucet_session = SessionState {
            scopes_bits: SCOPE_FAUCET,
            ..session.clone()
        };
        assert!(faucet_session.has_scope(&PermitAction::Faucet {
            market_id: 1,
            amount: 1_000,
            recipient: Pubkey::new_unique(),
        }));
    }

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
        let secret_key = SecretKey::from_bytes(&[7u8; 32]).unwrap();
        let public_key = PublicKey::from(&secret_key);
        let keypair = Ed25519Keypair {
            secret: secret_key,
            public: public_key,
        };

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
}
