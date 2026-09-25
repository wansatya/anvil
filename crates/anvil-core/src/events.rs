//! Event architecture (SPEC §26).
//! Agent -> TUI via `AgentEvent`, TUI -> Agent via `UserEvent`.

use serde::{Deserialize, Serialize};

/// A tool invocation requested by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
    /// Human-readable one-line summary, e.g. `read_file src/can/parser.cpp`.
    pub summary: String,
}

impl ToolCall {
    pub fn new(id: &str, name: &str, arguments: serde_json::Value, summary: &str) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            arguments,
            summary: summary.to_string(),
        }
    }
}

/// Result of a tool execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub success: bool,
    /// Short one-line outcome for the transcript, e.g. `✓ 312 lines`.
    pub summary: String,
    pub output: String,
}

/// Approval request for dangerous tools (SPEC §12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub tool_call: ToolCall,
    /// Command / description shown in the approval dialog.
    pub preview: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    AllowOnce,
    AllowAlways,
    Deny,
}

/// Events emitted by the agent runtime toward the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// Agent started thinking / contacting the model.
    Thinking,
    /// A streamed text fragment from the model.
    TextDelta(String),
    /// A complete assistant message boundary (optional, for tests).
    TextDone,
    ToolStarted(ToolCall),
    ToolOutput(ToolResult),
    ApprovalRequired(ApprovalRequest),
    Completed,
    Cancelled,
    Error(String),
    /// Usage line update: `model: cpp-agent tokens: 8.2k latency: 1.8s`.
    Usage { status: String },
    /// Local (non-model) output surfaced in the transcript, e.g. `/diff`.
    Note(String),
}

/// Events from the TUI toward the agent runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserEvent {
    Message(String),
    Cancel,
    Approval(ApprovalDecision),
    Quit,
}
