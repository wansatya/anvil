//! Provider-independent message model (SPEC §9).
//! Never pass raw provider JSON through the application.

use serde::{Deserialize, Serialize};

/// A tool call requested by the model (normalized).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Assistant content: text and/or tool calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub content: Option<String>,
    pub tool_calls: Vec<RequestedToolCall>,
}

/// Tool execution result fed back to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolMessage {
    pub tool_call_id: String,
    pub content: String,
}

/// One conversation message in normalized form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    System(String),
    User(String),
    Assistant(AssistantMessage),
    Tool(ToolMessage),
}

impl Message {
    pub fn role(&self) -> &'static str {
        match self {
            Message::System(_) => "system",
            Message::User(_) => "user",
            Message::Assistant(_) => "assistant",
            Message::Tool(_) => "tool",
        }
    }
}

/// A tool exposed to the model: name, description, JSON Schema parameters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    pub fn new(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        }
    }
}
