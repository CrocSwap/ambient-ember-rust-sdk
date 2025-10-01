use ambient_signer::{
    PermitEnvelopeV1, PermitDomain, ClusterType, PermitAction, 
    KeyType, ReplayMode, TimeInForce, sign_permit_ed25519, bytes_to_hex
};
use borsh::to_vec;
use solana_sdk::{
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    signer::SeedDerivable,
};
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Signature Checker");
    println!("=====================================================================");
    println!();

    // EXACT parameters from a successful order
    let market_id = 64u64; // BTC Market ID
    let client_id = 1759303703759u128; // Same Client ID/ Nonce
    let side = 0u8; // Bid
    let qty = 6000u64; // 0.00006000 * 10^8
    let price = 108000000000u64; // 108000.0 * 10^6
    let nonce = 1759303703759u64; // Same Nonce as from time of txn
    let expires_unix = 1759303763i64; // Expiry time = nonce/1000 + 60 
    let reduce_only = false;
    let tif = TimeInForce::GTC;

    println!("- Test Parameters:");
    println!("   Market ID: {}", market_id);
    println!("   Client ID: {}", client_id);
    println!("   Side: {} (Bid)", side);
    println!("   Quantity: {} scaled units", qty);
    println!("   Price: {} scaled units", price);
    println!("   Nonce: {}", nonce);
    println!("   Expires Unix: {}", expires_unix);
    println!("   Reduce Only: {}", reduce_only);
    println!("   Time in Force: {:?}", tif);
    println!();

    // Note: Expected signature will vary based on timestamp, so we verify structure too
    println!();

    // Read the keypair from devkey.json (same as the one used in the successful txn)
    let keypair_path = "devkey.json";
    println!("- Reading keypair from: {}", keypair_path);
    
    let keypair_json = std::fs::read_to_string(keypair_path)
        .map_err(|e| format!("Failed to read keypair file {}: {}. Make sure you've run 'solana-keygen new --outfile devkey.json'", keypair_path, e))?;
    
    // Parse the JSON array of bytes
    let keypair_bytes: Vec<u8> = serde_json::from_str(&keypair_json)
        .map_err(|e| format!("Failed to parse keypair JSON: {}", e))?;
    
    if keypair_bytes.len() != 64 {
        return Err(format!("Invalid keypair length: expected 64 bytes, got {}", keypair_bytes.len()).into());
    }

    // Create Solana keypair from the keypair bytes (first 32 bytes are the seed)
    let keypair = Keypair::from_seed(&keypair_bytes[..32])?;

    println!("✅ Loaded keypair from devkey.json successfully");
    println!("🔑 Using keypair:");
    println!("   Public key: {}", keypair.pubkey().to_string());
    println!("   Secret key: {}", bytes_to_hex(&keypair_bytes[..32]));
    println!();

    // Create the permit envelope with EXACT same structure as the txn
    let permit_envelope = PermitEnvelopeV1 {
        domain: PermitDomain {
            program_id: Pubkey::from_str("6egfvA3boGA8BLTgCzwPfKZMv3W9QS5V61Ewqa6VWq2g")?,  // Ambient Finance Program ID
            cluster: ClusterType::Testnet,
            version: 1,
        },
        authorizer: keypair.pubkey(),
        key_type: KeyType::Ed25519,
        action: PermitAction::Place {
            market_id,
            client_id,
            side,
            qty,
            price: Some(price),
            reduce_only,
            tif,
            trigger_price: None,
            trigger_type: 0,
            health_floor: None,
        },
        mode: ReplayMode::HlWindow { k: 128 },
        expires_unix,
        max_fee_quote: 1000000, // 1 USDC max fee 
        relayer: None,
        nonce,
    };

    println!("- PERMIT STRUCTURE VERIFICATION:");
    println!("- Domain:");
    println!("   Program ID: {}", permit_envelope.domain.program_id);
    println!("   Cluster: {:?}", permit_envelope.domain.cluster);
    println!("   Version: {}", permit_envelope.domain.version);
    println!("- Authorizer: {}", permit_envelope.authorizer);
    println!("- Key Type: {:?}", permit_envelope.key_type);
    println!("- Action Details:");
    if let PermitAction::Place { 
        market_id, client_id, side, qty, price, reduce_only, tif, 
        trigger_price, trigger_type, health_floor 
    } = &permit_envelope.action {
        println!("   Market ID: {}", market_id);
        println!("   Client ID: {}", client_id);
        println!("   Side: {}", side);
        println!("   Quantity: {}", qty);
        println!("   Price: {:?}", price);
        println!("   Reduce Only: {}", reduce_only);
        println!("   Time in Force: {:?}", tif);
        println!("   Trigger Price: {:?}", trigger_price);
        println!("   Trigger Type: {}", trigger_type);
        println!("   Health Floor: {:?}", health_floor);
    }
    println!("- Mode: {:?}", permit_envelope.mode);
    println!("- Expires Unix: {}", permit_envelope.expires_unix);
    println!("- Max Fee Quote: {} (minimal)", permit_envelope.max_fee_quote);
    println!("- Relayer: {:?} (minimal)", permit_envelope.relayer);
    println!("- Nonce: {}", permit_envelope.nonce);
    println!();

    // Serialize the permit to see the exact bytes being signed
    let permit_bytes = to_vec(&permit_envelope)?;
    println!("- Permit Serialization:");
    println!("   Length: {} bytes", permit_bytes.len());
    println!("   Hex: {}", bytes_to_hex(&permit_bytes));
    println!();

    // Break down the serialization to understand the structure
    println!("- DETAILED SERIALIZATION BREAKDOWN:");
    let mut offset = 0;
    
    // Domain serialization
    let domain_bytes = to_vec(&permit_envelope.domain)?;
    println!("   Domain ({} bytes): {}", domain_bytes.len(), bytes_to_hex(&domain_bytes));
    offset += domain_bytes.len();
    
    // Authorizer (32 bytes)
    let authorizer_bytes = permit_envelope.authorizer.to_bytes();
    println!("   Authorizer ({} bytes): {}", authorizer_bytes.len(), bytes_to_hex(&authorizer_bytes));
    offset += authorizer_bytes.len();
    
    // Key type (1 byte)
    let key_type_bytes = to_vec(&permit_envelope.key_type)?;
    println!("   Key Type ({} bytes): {}", key_type_bytes.len(), bytes_to_hex(&key_type_bytes));
    offset += key_type_bytes.len();
    
    // Action
    let action_bytes = to_vec(&permit_envelope.action)?;
    println!("   Action ({} bytes): {}", action_bytes.len(), bytes_to_hex(&action_bytes));
    offset += action_bytes.len();
    
    // Mode
    let mode_bytes = to_vec(&permit_envelope.mode)?;
    println!("   Mode ({} bytes): {}", mode_bytes.len(), bytes_to_hex(&mode_bytes));
    offset += mode_bytes.len();
    
    // Expires unix (8 bytes)
    let expires_bytes = to_vec(&permit_envelope.expires_unix)?;
    println!("   Expires Unix ({} bytes): {}", expires_bytes.len(), bytes_to_hex(&expires_bytes));
    offset += expires_bytes.len();
    
    // Max fee quote (8 bytes)
    let max_fee_bytes = to_vec(&permit_envelope.max_fee_quote)?;
    println!("   Max Fee Quote ({} bytes): {}", max_fee_bytes.len(), bytes_to_hex(&max_fee_bytes));
    offset += max_fee_bytes.len();
    
    // Relayer (Option, 1 byte for None)
    let relayer_bytes = to_vec(&permit_envelope.relayer)?;
    println!("   Relayer ({} bytes): {}", relayer_bytes.len(), bytes_to_hex(&relayer_bytes));
    offset += relayer_bytes.len();
    
    // Nonce (8 bytes)
    let nonce_bytes = to_vec(&permit_envelope.nonce)?;
    println!("   Nonce ({} bytes): {}", nonce_bytes.len(), bytes_to_hex(&nonce_bytes));
    offset += nonce_bytes.len();
    
    println!("   Total: {} bytes", offset);
    println!();

    // Sign the permit
    let signed_permit = sign_permit_ed25519(&permit_envelope, &keypair)?;
    let (payload, signature) = signed_permit.into_parts();
    let signature_hex = bytes_to_hex(&signature);

    println!("-  SIGNATURE RESULTS:");
    println!("   Payload length: {} bytes", payload.len());
    println!("   Payload hex: {}", bytes_to_hex(&payload));
    println!("   Signature length: {} bytes", signature.len());
    println!("   Signature hex: {}", signature_hex);
    println!();

    // Verify signature structure
    println!("✅ SIGNATURE VERIFICATION:");
    println!("   Generated: {}", signature_hex);
    println!("   ✅ Signature generated successfully with {} bytes", signature.len());

    // Verify signature cryptographically using Solana SDK
    use solana_sdk::signature::Signature;
    let sig_obj = Signature::from(signature);
    match sig_obj.verify(keypair.pubkey().as_ref(), &payload) {
        true => println!("   ✅ Cryptographic signature verification: PASSED"),
        false => println!("   ❌ Cryptographic signature verification: FAILED"),
    }

    Ok(())
}