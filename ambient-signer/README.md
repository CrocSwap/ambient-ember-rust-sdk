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

- `borsh = "0.9.3"` - For serialization
- `solana-program = "1.18.26"` - For Pubkey and program utilities  
- `ed25519-dalek = "1.0"` - For Ed25519 signing (optional, behind `permit-signing` feature)
- `hex = "0.4.3"` - For hex string utilities
- `rand = "0.7"` - For keypair generation
- `serde_json = "1.0"` - For parsing Solana keypair JSON files

## Features

- `permit-signing` (default): Enables Ed25519 signing functionality

## License

MIT OR Apache-2.0

## Tests:

1. Place a `devkey.json` file in the parent directory with your secret key
2. `cd ambient-ember-rust-sdk`
3. `cargo run --bin signature_checker`: Verifies using custom params if our signer is able to generate a matching signature against a given signature when the same params from a successful transaction are given.
4. `cargo run --bin test_signature_and_place_order`: Signs a permit, creates a signature and sends a signed payload to the exchange endpoint to place an order and verifies the success of the order.
