use anyhow::Result;
use rig::completion::{CompletionModel, CompletionRequest, CompletionResponse};
use rig::providers::openai::CompletionModel as OpenAIModel;
use crate::antigravity::AntigravityCompletionModel;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Enum wrapping all supported concrete completion models
#[derive(Clone)]
pub enum MetaInnerModel {
    Antigravity(AntigravityCompletionModel),
    OpenAI(OpenAIModel),
    // Add other providers here (Anthropic, etc.)
}

/// MetaCompletionModel that implements CompletionModel and delegates to inner
#[derive(Clone)]
pub struct MetaCompletionModel {
    pub chain: Vec<MetaInnerModel>,
    pub current_index: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

pub struct MetaResponse {
    pub content: String,
}

// We need a unified response type. 
// Provide a simple one for now that just holds the string content.
impl MetaResponse {
    pub fn new(content: String) -> Self {
        Self { content }
    }
}

// To implement CompletionModel, we need to match the associated types.
// Antigravity uses String response. OpenAI uses ChatResponse.
// This is tricky because CompletionModel trait requires a specific Response type.
// If rig's CompletionModel enforces a specific structure, we might be stuck.
// Let's look at rig's definition again (inferred).
// AntigravityCompletionModel::Response = String.
// OpenAIModel::Response = ChatResponse (struct).

// CRITICAL: We cannot implement CompletionModel for MetaCompletionModel if inner models return different types!
// Unless we define our own UnifiedResponse and impl CompletionModel for it?
// content: String is common.

// Let's pause. If AgentLoop expects M: CompletionModel, does it use the Response type?
// It likely calls `completion()` and gets `Response`. Then it extracts content.
// If M::Response varies, AgentLoop code might be generic over it? Or expects specific structure?

// If AgentLoop uses `agent.chat()`, `rig` handles the response parsing.

// Solution: We might need to wrap OpenAI model to return String response like Antigravity does.
// Or wrap Antigravity to return OpenAI response?

// Let's create a wrapper for OpenAI that normalizes it to String response.
// OpenAIModel returns ChatResponse.

// Actually, `rig` Agents are usually built with specific model types.
// If we want dynamic switching, we need `MetaCompletionModel` to return a `UnifiedResponse`.
// Does `rig` Agent support custom Response types? Yes, associated type.

// So, MetaCompletionModel::Response = String.
// Antigravity returns String. Good.
// OpenAI returns ChatResponse. We need to map it to String.

// How? We can't change OpenAIModel's return type.
// But we can wrap OpenAIModel in a new struct `OpenAIStringModel` that calls inner and converts response.

// Plan:
// 1. Define `OpenAIStringModel` wrapping `rig::providers::openai::CompletionModel`.
// 2. Impl `CompletionModel` for it, traversing `Response` -> `String`.
// 3. `MetaInnerModel` wraps `Antigravity` and `OpenAIStringModel`.
// 4. `MetaCompletionModel` delegates.

// This unifies the return type to `String`.

#[derive(Clone)]
pub struct OpenAIStringModel(pub OpenAIModel);

// We need to implement CompletionModel for OpenAIStringModel...
// But we can't impl trait for external type easily if not defining it.
// Struct is local so it's fine.

// Let's write the file.
