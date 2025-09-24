# Ember Core Crate

`ember-core` houses the shared data models, serialization logic, and utility
functions that power the Ambient Ember Solana program, keepers, indexers, and
SDKs. Everything in this crate is designed to be Borsh-compatible so the same
types can flow across on-chain and off-chain components without manual byte
packing.

## What Lives Here

- **State Models** – account layouts, orderbook types, and math helpers that are
  shared between the program and off-chain services.
- **Instruction Definitions** – discriminants and payload structures for every
  on-chain instruction, keeping serialization consistent.
- **Permit System** – domain, envelope, and replay-protection structures that
  encode off-chain intents for trustless submission.

## Using the Permit System

Permits let clients authorize actions (place orders, cancel, withdraw, etc.)
off-chain and hand them to a relayer or keeper without exposing their private
keys. Each permit is described by a `PermitEnvelopeV1` value consisting of a
domain, action payload, replay-mode, expiry, and fee limits. The lower-level
types live in `core/src/permit.rs` and are shared with the on-chain logic.

### Serializing Permit Bytes

```rust
use ember_core::permit::{PermitEnvelopeV1, PermitAction, PermitDomain, ReplayMode, ClusterType};
use solana_program::pubkey::Pubkey;

let envelope = PermitEnvelopeV1 {
    domain: PermitDomain {
        program_id: Pubkey::from_str("6egfvA3boGA8BLTgCzwPfKZMv3W9QS5V61Ewqa6VWq2g").unwrap(),
        cluster: ClusterType::Testnet,
        version: 1,
    },
    authorizer: Pubkey::new_unique(),
    key_type: ember_core::permit::KeyType::Ed25519,
    action: PermitAction::Noop,
    mode: ReplayMode::HlWindow { k: 128 },
    expires_unix: 1_700_000_000,
    max_fee_quote: 0,
    relayer: None,
    nonce: 42,
};

let permit_bytes = envelope.try_to_vec()?; // Borsh serialization shared with the program
```

### Signing Permits in Rust

The crate ships an optional signing helper behind the `permit-signing` feature.
It uses `ed25519-dalek` to produce the 64-byte Ed25519 signature expected by the
program’s signature verification.

```toml
# core/Cargo.toml or your dependent crate
ember-core = { path = "../core", features = ["permit-signing"] }
```

```rust
use ed25519_dalek::Keypair;
use ember_core::permit::{sign_permit_ed25519, PermitEnvelopeV1};

let keypair: Keypair = /* load or derive */;
let envelope: PermitEnvelopeV1 = /* build as above */;

let signed = sign_permit_ed25519(&envelope, &keypair)?;
let (bytes, signature) = signed.into_parts();

// bytes -> submit in transaction data
// signature -> pass to the program's ed25519 verification ix
```

The helper simply serializes the envelope with Borsh and signs the resulting
bytes. Keeping the process deterministic ensures Node/TS clients, Rust keepers,
and the on-chain verifier all agree on the payload.

## Testing

Run the full unit suite:

```bash
cargo test -p ember-core
```

To exercise the permit signing helper:

```bash
cargo test -p ember-core --features permit-signing
```

## Regenerating Permit Fixtures

The SDK keeps JSON fixtures aligned with the Rust permit definitions. After any
changes to `core/src/permit.rs`, regenerate them:

```bash
cargo run -p permit-fixtures
(cd sdk && pnpm test tests/permit.spec.ts)
```

This guarantees off-chain clients match the serialized bytes and action schema
used on-chain.
