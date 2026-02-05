use anyhow::Result;
use serde_json::Value; // Added to parse JSON response
use std::fs;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    // Load token
    let home = dirs::home_dir().expect("no home");
    let token_path = home.join(".nanobot").join("tokens.json");
    let content = fs::read_to_string(token_path)?;
    let json: Value = serde_json::from_str(&content)?;
    
    let token = json["tokens"]["antigravity"]["access_token"]
        .as_str()
        .expect("no token")
        .to_string();

    println!("Found token: {}...", &token[0..10]);

    let client = reqwest::Client::new();
    
    // Test Models
    let models = vec![
        "gemini-3-flash-preview",
        "gemini-2.0-flash", 
        "gemini-1.5-flash",
    ];

    // OpenAI Compatible Endpoint
    let base_url = "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions";

    for model in models {
        println!("\nTesting model: {}", model);
        
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "user", "content": "Hello"}
            ],
            "max_tokens": 10
        });

        let res = client.post(base_url)
            .header("Authorization", format!("Bearer {}", token))
            .json(&body)
            .send()
            .await?;

        println!("Status: {}", res.status());
        let text = res.text().await?;
        println!("Response: {}", text);
    }

    Ok(())
}
