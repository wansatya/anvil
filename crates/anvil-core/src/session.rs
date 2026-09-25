//! Session / conversation state (SPEC §19).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
    System,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: &str) -> Self {
        Self { role: MessageRole::User, content: content.to_string() }
    }
    pub fn assistant(content: &str) -> Self {
        Self { role: MessageRole::Assistant, content: content.to_string() }
    }
    pub fn tool(content: &str) -> Self {
        Self { role: MessageRole::Tool, content: content.to_string() }
    }
    pub fn system(content: &str) -> Self {
        Self { role: MessageRole::System, content: content.to_string() }
    }
    pub fn error(content: &str) -> Self {
        Self { role: MessageRole::Error, content: content.to_string() }
    }
}

/// In-memory session; persistence (JSONL on disk) lands in Phase 8.
#[derive(Debug, Default, Clone)]
pub struct SessionState {
    pub id: String,
    pub model: String,
    pub messages: Vec<ChatMessage>,
}

impl SessionState {
    pub fn new(id: &str, model: &str) -> Self {
        Self { id: id.to_string(), model: model.to_string(), messages: Vec::new() }
    }

    pub fn push(&mut self, msg: ChatMessage) {
        self.messages.push(msg);
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_push_and_clear() {
        let mut s = SessionState::new("s1", "cpp-agent");
        s.push(ChatMessage::user("hi"));
        assert_eq!(s.messages.len(), 1);
        s.clear();
        assert!(s.messages.is_empty());
    }
}
