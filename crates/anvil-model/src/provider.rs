//! Provider trait + streaming types (SPEC §7).

use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::types::{Message, ToolSpec};

/// Typed model errors (SPEC §27). The TUI renders `user_message()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    Transport(String),
    Auth(String),
    RateLimited(String),
    BadRequest(String),
    Parse(String),
    Cancelled,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelError::Transport(e) => write!(f, "model transport error: {e}"),
            ModelError::Auth(e) => write!(f, "model auth error: {e}"),
            ModelError::RateLimited(e) => write!(f, "model rate limited: {e}"),
            ModelError::BadRequest(e) => write!(f, "model rejected request: {e}"),
            ModelError::Parse(e) => write!(f, "model response parse error: {e}"),
            ModelError::Cancelled => write!(f, "model request cancelled"),
        }
    }
}

impl std::error::Error for ModelError {}

/// One item of a streaming chat response.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    TextDelta(String),
    ToolCall(crate::types::RequestedToolCall),
    Usage { input_tokens: u64, output_tokens: u64 },
    Done,
}

/// An open response stream. The provider spawns the HTTP work and pushes
/// events; dropping the receiver cancels consumption.
pub type ChatStream = mpsc::UnboundedReceiver<Result<StreamEvent, ModelError>>;

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub temperature: f32,
    pub max_tokens: u32,
}

impl ChatRequest {
    pub fn new(model: &str, messages: Vec<Message>) -> Self {
        Self {
            model: model.to_string(),
            messages,
            tools: Vec::new(),
            temperature: 0.2,
            max_tokens: 4096,
        }
    }
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    /// Open a streaming chat completion. `cancel` flips to true on user
    /// cancellation (SPEC §22); providers must stop promptly.
    async fn chat(
        &self,
        request: ChatRequest,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<ChatStream, ModelError>;
}
