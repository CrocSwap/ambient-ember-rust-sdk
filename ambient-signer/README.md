# Ambient Signer

A minimal dependencies crate for signing and sending payloads to Ambient Exchange. This crate is extracted from the core permit system and provides only the essential functionality needed for signing operations.

## Features

- **Minimal Dependencies**: Only includes necessary dependencies for signing operations
- **Ed25519 Signing**: Support for Ed25519 signature generation with optional feature
- **Borsh Serialization**: Uses Borsh for deterministic serialization
- **Hex Utilities**: Helper functions for hex string conversion
- **Compatible**: Works with Solana ecosystem (solana-program)

## Usage

Add this to your `Cargo.toml`:

```toml
[dependencies]
ambient-signer = { path = "../ambient-signer" }
```

## Dependencies

**Core (minimal):**
- `borsh = "1.5.7"` - For serialization
- `solana-sdk = "1.18.26"` - For Pubkey and signing utilities  
- `hex = "0.4.3"` - For hex string utilities

**Optional (testing only):**
- `serde_json = "1.0"` - For parsing Solana keypair JSON files
- `reqwest = "0.11"` - For HTTP requests to exchange API
- `tokio = "1.0"` - For async runtime

## Features

- `testing`: Enables additional dependencies needed for test binaries and examples

## License

MIT OR Apache-2.0

## Tests:

1. Place a `devkey.json` file in the parent directory with your secret key
2. `cd ambient-ember-rust-sdk`
3. `cargo run --bin signature_checker --features testing`: Verifies using custom params if our signer is able to generate a matching signature against a given signature when the same params from a successful transaction are given.
4. `cargo run --bin test_signature_and_place_order --features testing`: Signs a permit, creates a signature and sends a signed payload to the exchange endpoint to place an order and verifies the success of the order.

**Note:** The `--features testing` flag is required to enable the optional dependencies (serde_json, reqwest, tokio) needed by the test binaries.
