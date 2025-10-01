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
use std::fs;
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

// API endpoint configuration
const AMBIENT_API_URL: &str = "https://embindexer.net/ember/api/dev/v1/exchange";


async fn send_order_request(
    client: &Client,
    pubkey: &str,
    nonce: u64,
    signature_hex: &str,
    client_order_id: u64,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let request_body = json!({
        "action": {
            "type": "order",
            "a": 0,  // asset ID (BTC)
            "b": true,  // isBuy (true for bid)
            "p": "108000.0",  // price 
            "s": "0.00006000",  // size
            "r": false,  // reduceOnly
            "t": {
                "limit": {
                    "tif": "Gtc"  // Good Till Cancel
                }
            },
            "c": client_order_id.to_string()  // cloid as string
        },
        "nonce": nonce,
        "signature": [signature_hex],  // Array format
        "pubkey": pubkey 
    });

    println!("Sending order request to: {}", AMBIENT_API_URL);
    println!("Request body: {}", serde_json::to_string_pretty(&request_body)?);

    let response = client
        .post(AMBIENT_API_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", "ambient-signer-test/1.0")
        .json(&request_body)
        .timeout(Duration::from_secs(30))
        .send()
        .await?;

    let status = response.status();
    let response_text = response.text().await?;

    println!("Response status: {}", status);
    println!("Response body: {}", response_text);

    if status.is_success() {
        let json_response: serde_json::Value = serde_json::from_str(&response_text)
            .unwrap_or_else(|_| json!({"raw_response": response_text}));
        Ok(json_response)
    } else {
        Err(format!("API request failed with status {}: {}", status, response_text).into())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Testing ambient-signer with real Solana keypair...\n");

    // Read the keypair from the JSON file containing the key
    let keypair_path = "devkey.json";
    println!("Reading keypair from: {}", keypair_path);
    
    let keypair_json = fs::read_to_string(keypair_path)
        .map_err(|e| format!("Failed to read keypair file {}: {}. Make sure you've run 'solana-keygen new --outfile devkey.json'", keypair_path, e))?;
    
    // Parse the JSON array of bytes
    let keypair_bytes: Vec<u8> = serde_json::from_str(&keypair_json)
        .map_err(|e| format!("Failed to parse keypair JSON: {}", e))?;
    
    if keypair_bytes.len() != 64 {
        return Err(format!("Invalid keypair length: expected 64 bytes, got {}", keypair_bytes.len()).into());
    }

    // Create Solana keypair from the keypair bytes (first 32 bytes are the seed)
    let keypair = Keypair::from_seed(&keypair_bytes[..32])?;
    
    println!("✅ Loaded keypair successfully");
    println!("📋 Public key: {}", keypair.pubkey().to_string());
    println!();

    // Get current timestamp for nonce and client order ID
    let current_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    
    // Python script scaling: SIZE_DECIMALS = 8, PRICE_DECIMALS = 6
    let size_scaled = (0.00006000 * 10_f64.powi(8)) as u64; // 6000 scaled units
    let price_scaled = (108000.0 * 10_f64.powi(6)) as u64; // 108000000000 scaled units
    
    println!("Creating order placement permit...");
    println!("Scaled values: size={}, price={}", size_scaled, price_scaled);
    
    let order_envelope = PermitEnvelopeV1 {
        domain: PermitDomain {
            program_id: Pubkey::from_str("6egfvA3boGA8BLTgCzwPfKZMv3W9QS5V61Ewqa6VWq2g")?, // Program ID for Ambient Finance
            cluster: ClusterType::Testnet,
            version: 1,
        },
        authorizer: keypair.pubkey(),
        key_type: KeyType::Ed25519,
        action: PermitAction::Place {
            market_id: 64,
            client_id: current_timestamp as u128, // Use timestamp as client_id
            side: 0, // Bid
            qty: size_scaled, // Scaled size: 0.00006000 * 10^8 = 6000
            price: Some(price_scaled), // Scaled price: 108000.0 * 10^6 = 108000000000
            reduce_only: false,
            tif: TimeInForce::GTC,
            trigger_price: None,
            trigger_type: 0,
            health_floor: None,
        },
        mode: ReplayMode::HlWindow { k: 128 },
        expires_unix: (current_timestamp / 1000 + 60) as i64, // 60 seconds after nonce
        max_fee_quote: 1000000, // Max 1 USDC fee
        relayer: None,
        nonce: current_timestamp, // Use timestamp as nonce
    };

    println!("- Order details:");
    println!("   Market ID: 64");
    println!("   Client ID: {} (timestamp)", current_timestamp);
    println!("   Side: Bid");
    println!("   Quantity: {} scaled units (0.00006000 BTC)", size_scaled);
    println!("   Price: {} scaled units ($108,000.00)", price_scaled);
    println!("   Time in Force: GTC (Good Till Cancel)");
    println!("   Expires: {} (Unix timestamp)", order_envelope.expires_unix);
    println!("   Nonce: {} (matches client_id)", order_envelope.nonce);
    println!();

    println!("- PERMIT ENVELOPE DETAILS:");
    println!("- Domain:");
    println!("   Program ID: {}", order_envelope.domain.program_id);
    println!("   Cluster: {:?}", order_envelope.domain.cluster);
    println!("   Version: {}", order_envelope.domain.version);
    println!("- Authorizer: {}", order_envelope.authorizer);
    println!("- Key Type: {:?}", order_envelope.key_type);
    println!("- Action: {:?}", order_envelope.action);
    println!("- Mode: {:?}", order_envelope.mode);
    println!("- Expires Unix: {}", order_envelope.expires_unix);
    println!("- Max Fee Quote: {} (1 USDC)", order_envelope.max_fee_quote);
    println!("- Relayer: {:?}", order_envelope.relayer);
    println!("- Nonce: {}", order_envelope.nonce);
    println!();

    // Serialize the permit to show what gets signed
    let permit_bytes = to_vec(&order_envelope)?;
    println!("- Permit serialized to {} bytes:", permit_bytes.len());
    println!("- Permit bytes (hex): {}", bytes_to_hex(&permit_bytes));
    println!();

    // Sign the order permit
    let order_signed = sign_permit_ed25519(&order_envelope, &keypair)?;
    let (order_payload, order_signature) = order_signed.into_parts();

    println!("- Successfully signed order placement permit!");
    println!();
    
    // Step 3: Display both transactions ready for /exchange
    println!("Ready to send order to /exchange endpoint:");
    println!();
    println!("BUY ORDER - BTC at $108,000 (Size: 0.00006000):");
    println!("   - Payload length: {} bytes", order_payload.len());
    println!("   - Payload (hex): {}", bytes_to_hex(&order_payload));
    println!("   -  Signature length: {} bytes", order_signature.len());
    println!("   -  Signature (hex): {}", bytes_to_hex(&order_signature));
    println!();

    // Verify signature to ensure it's correct
    use solana_sdk::signature::Signature;
    
    let order_sig_obj = Signature::from(order_signature);
    match order_sig_obj.verify(keypair.pubkey().as_ref(), &order_payload) {
        true => println!("- Order signature verification: PASSED"),
        false => println!("- Order signature verification: FAILED"),
    }

    println!();
    println!("Step 3: Sending order to Ambient Exchange API...");
    println!();

    // Create HTTP client
    let client = Client::new();

    // Convert public key to Solana base58 format
    let pubkey_string = keypair.pubkey().to_string();

    // Send order placement transaction (skip faucet since devkey is already funded)
    println!("- Sending order placement transaction...");
    println!(" Using funded devkey.json wallet");
    println!(" Using nonce/client_id: {}", current_timestamp);
    match send_order_request(
        &client,
        &pubkey_string,
        current_timestamp, // nonce matches client_id
        &bytes_to_hex(&order_signature),
        current_timestamp // client_order_id matches nonce
    ).await {
        Ok(response) => {
            println!("- Order placement transaction sent successfully!");
            println!(" - Client Order ID: {}", current_timestamp);
            println!(" - Response: {}", serde_json::to_string_pretty(&response)?);
            
            // Check if order was placed successfully
            if let Some(status) = response.get("status") {
                if status == "ok" {
                    if let Some(response_data) = response.get("response") {
                        if let Some(data) = response_data.get("data") {
                            if let Some(oid) = data.get("oid") {
                                println!("ORDER PLACED SUCCESSFULLY");
                                println!("- Order ID (OID): {}", oid);
                                if let Some(tx) = response_data.get("tx") {
                                    println!("- Transaction: {}", tx);
                                }
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            println!("Order placement transaction failed: {}", e);
        }
    }

    Ok(())
}