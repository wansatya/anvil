//! Anvil core: agent runtime types, events, session state.
//! Keeps the runtime testable without the TUI (SPEC §26).

pub mod agent;
pub mod context;
pub mod events;
pub mod permissions;
pub mod session;

pub use agent::{Agent, AgentConfig, AgentError, AgentOutcome, ApprovalGate, ApprovalJob};
pub use context::Workspace;
pub use events::{AgentEvent, ApprovalDecision, ApprovalRequest, ToolCall, ToolResult, UserEvent};
pub use permissions::{PermissionSet, Policy};
pub use session::{ChatMessage, MessageRole, SessionState};

/// Default base model: Muse Spark 1.3 (free) via the OpenCode Zen API,
/// consumed as an OpenAI-compatible provider (SPEC §8):
/// `POST {base_url}/chat/completions`.
pub const DEFAULT_MODEL: &str = "opencode/muse-spark-1.3-contributor-free";
/// Zen gateway root; chat completions live at `/chat/completions`.
pub const DEFAULT_BASE_URL: &str = "https://opencode.ai/zen/v1";
