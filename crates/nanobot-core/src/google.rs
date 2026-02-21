use anyhow::Result;
use reqwest::Client;
use rig::OneOrMany;
use rig::completion::message::{AssistantContent, UserContent};
use rig::completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse};
use rig::streaming::StreamingCompletionResponse;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone)]
pub struct GoogleCompletionModel {
    pub client: Client,
    pub api_key: String,
    pub model: String,
}

impl CompletionModel for GoogleCompletionModel {
    type Response = String;
    type StreamingResponse = GoogleStreamingResponse;
    type Client = Client;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self {
            client: client.clone(),
            api_key: String::new(), // Will be set by caller
            model: model.into(),
        }
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let body = build_gemini_request(&request);

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "Google API Error: {}",
                error_text
            )));
        }

        let json_resp: serde_json::Value = response
            .json()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        // Extract text from the first candidate
        let text = json_resp["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| CompletionError::ResponseError("No text in response".to_string()))?
            .to_string();

        Ok(CompletionResponse {
            choice: OneOrMany::one(AssistantContent::text(text.clone())),
            usage: rig::completion::Usage::default(),
            raw_response: text,
        })
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            self.model, self.api_key
        );

        let body = build_gemini_request(&request);

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| CompletionError::ProviderError(e.to_string()))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(CompletionError::ProviderError(format!(
                "Google API Error: {}",
                error_text
            )));
        }

        let stream = response.bytes_stream();

        use eventsource_stream::Eventsource;
        use futures::StreamExt;

        let event_stream = stream.eventsource();

        // Map SEE events to GoogleStreamingResponse
        let mapped_stream = event_stream.map(|event_res| {
            match event_res {
                Ok(event) => {
                    let data = event.data;
                    if data == "[DONE]" {
                        // Rig expects a Final variant to close stream properly, but empty message works too
                        return Ok(rig::streaming::RawStreamingChoice::Message("".to_string()));
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&data)
                        && let Some(text) =
                            json["candidates"][0]["content"]["parts"][0]["text"].as_str()
                    {
                        return Ok(rig::streaming::RawStreamingChoice::Message(
                            text.to_string(),
                        ));
                    }
                    // Fallback/Ignore empty
                    Ok(rig::streaming::RawStreamingChoice::Message("".to_string()))
                }
                Err(e) => Err(CompletionError::ProviderError(e.to_string())),
            }
        });

        Ok(StreamingCompletionResponse::stream(Box::pin(mapped_stream)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GoogleStreamingResponse {
    pub content: String,
}

impl rig::completion::GetTokenUsage for GoogleStreamingResponse {
    fn token_usage(&self) -> Option<rig::completion::Usage> {
        None
    }
}

impl From<String> for GoogleStreamingResponse {
    fn from(content: String) -> Self {
        Self { content }
    }
}

// Helper to build Gemini JSON request from Rig CompletionRequest
fn build_gemini_request(request: &CompletionRequest) -> serde_json::Value {
    // Construct Gemini "contents" from chat history
    let contents: Vec<serde_json::Value> = request
        .chat_history
        .iter()
        .map(|msg| {
            let role = match msg {
                rig::completion::Message::User { .. } => "user",
                rig::completion::Message::Assistant { .. } => "model",
            };

            let parts = match msg {
                rig::completion::Message::User { content } => {
                    content
                        .iter()
                        .map(|c| match c {
                            UserContent::Text(t) => json!({"text": t.text}),
                            _ => json!({"text": ""}), // Image not supported in this basic impl
                        })
                        .collect::<Vec<_>>()
                }
                rig::completion::Message::Assistant { content, .. } => content
                    .iter()
                    .map(|c| match c {
                        AssistantContent::Text(t) => json!({"text": t.text}),
                        _ => json!({"text": ""}),
                    })
                    .collect::<Vec<_>>(),
            };

            json!({
                "role": role,
                "parts": parts
            })
        })
        .collect();

    // Add system instruction if preamble exists
    let final_contents = contents;

    // Note: Gemini API puts system instruction adjacent to contents, not inside.
    let system_instruction = request.preamble.as_ref().map(|preamble| {
        json!({
            "parts": [{ "text": preamble }]
        })
    });

    let generation_config = json!({
        "temperature": request.temperature.unwrap_or(0.7),
        "maxOutputTokens": request.max_tokens,
    });

    json!({
        "contents": final_contents,
        "systemInstruction": system_instruction,
        "generationConfig": generation_config
    })
}
