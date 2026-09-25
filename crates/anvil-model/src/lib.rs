//! Model provider abstraction (SPEC §7-9).
//! The agent runtime only speaks these normalized types; provider wire
//! formats never leak past `anvil-model`.

pub mod mock;
pub mod openai;
pub mod provider;
pub mod types;

pub use mock::{MockProvider, ScriptedTurn};
pub use openai::OpenAiCompatible;
pub use provider::{ChatRequest, ChatStream, ModelError, ModelProvider, StreamEvent};
pub use types::{AssistantMessage, Message, RequestedToolCall, ToolMessage, ToolSpec};
