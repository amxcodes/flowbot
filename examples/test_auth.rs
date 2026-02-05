use anyhow::Result;
use serde_json::Value;
use std::fs;

#[tokio::main]
async fn main() -> Result<()> {
    // Load token to verify it exists
    let home = dirs::home_dir().expect("no home");
    let token_path = home.join(".nanobot").join("tokens.json");
    
    println!("Token path: {:?}", token_path);
    
    let content = fs::read_to_string(&token_path)?;
    let json: Value = serde_json::from_str(&content)?;
    
    let token = json["tokens"]["antigravity"]["access_token"]
        .as_str()
        .expect("no token")
        .to_string();

    println!("Found token: {}...", &token[0..20.min(token.len())]);
    println!("Token length: {}", token.len());

    // Test the actual endpoint
    let client = reqwest::Client::new();
    let base_url = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

    let body = serde_json::json!({
        "model": "gemini-2.0-flash-exp",
        "messages": [
            {"role": "system", "content": "You are a helpful assistant."},
            {"role": "user", "content": "Say hello!"}
        ],
        "max_tokens": 50
    });

    println!("\nTesting endpoint: {}", base_url);
    println!("Sending request...");

    let res = client.post(base_url)
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "nanobot-rs/0.1.0")
        .json(&body)
        .send()
        .await?;

    println!("Status: {}", res.status());
    
    let text = res.text().await?;
    println!("Response: {}", text);

    Ok(())
}
