use anyhow::Result;
use fastembed::{TextEmbedding, InitOptions};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub enum EmbeddingProvider {
    Local(Arc<Mutex<TextEmbedding>>),
    OpenAI { api_key: String, model: String },
}

impl EmbeddingProvider {
    pub fn local() -> Result<Self> {
        let model = TextEmbedding::try_new(InitOptions::default())?;
        Ok(Self::Local(Arc::new(Mutex::new(model))))
    }

    pub fn openai(api_key: String) -> Self {
        Self::OpenAI {
            api_key,
            model: "text-embedding-3-small".to_string(),
        }
    }

    pub async fn embed(&self, texts: Vec<&str>) -> Result<Vec<Vec<f32>>> {
        match self {
            Self::Local(model) => {
                let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
                // Lock mutex to get mutable access
                let mut model = model.lock().await;
                let embeddings = model.embed(owned, None)?;
                Ok(embeddings)
            }
            Self::OpenAI { api_key, model } => {
                let client = reqwest::Client::new();
                let response = client
                    .post("https://api.openai.com/v1/embeddings")
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("Content-Type", "application/json")
                    .json(&json!({
                        "model": model,
                        "input": texts,
                    }))
                    .send()
                    .await?;

                if !response.status().is_success() {
                    let error_text = response.text().await?;
                    return Err(anyhow::anyhow!(
                        "OpenAI embedding failed: {}",
                        error_text
                    ));
                }

                let json: serde_json::Value = response.json().await?;
                let data = json["data"]
                    .as_array()
                    .ok_or_else(|| anyhow::anyhow!("Invalid OpenAI response"))?;

                let mut embeddings = Vec::new();
                for entry in data {
                    let embedding = entry["embedding"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("Missing embedding in response"))?;
                    let vec: Vec<f32> = embedding
                        .iter()
                        .filter_map(|v| v.as_f64().map(|f| f as f32))
                        .collect();
                    embeddings.push(vec);
                }
                Ok(embeddings)
            }
        }
    }
}
