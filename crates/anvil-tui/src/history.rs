//! Session history persistence (SPEC §19): transcript snapshots as JSON.
//!
//! NOTE: this lives in `anvil-tui` (not `anvil-core`) only because the
//! persisted transcript type (`TranscriptEntry`) is owned by the TUI crate.
//! A Phase 8 follow-up may move it behind a core trait.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::app::TranscriptEntry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSession {
    pub version: u32,
    pub id: String,
    pub model: String,
    pub base_url: String,
    pub updated_at: u64,
    pub messages: Vec<TranscriptEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub id: String,
    pub model: String,
    pub messages: usize,
    pub updated_at: u64,
}

/// Session storage root. `ANVIL_DATA_DIR` overrides (used by tests),
/// otherwise `~/.local/share/anvil/sessions/` (SPEC §19).
pub fn sessions_dir() -> PathBuf {
    if let Ok(d) = std::env::var("ANVIL_DATA_DIR") {
        if !d.trim().is_empty() {
            return PathBuf::from(d).join("sessions");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/anvil/sessions")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn new_session_id() -> String {
    format!("{:x}-{}", now_secs(), std::process::id())
}

/// Cap so a long session can't grow the snapshot without bound.
const MAX_SAVED_MESSAGES: usize = 500;

/// Convert a stored transcript to normalized model messages for resuming.
/// Only conversation-carrying entries survive (user text, assistant text,
/// tool results); UI-only entries (tool starts, errors, system notes) are
/// skipped so the model never sees ids it can't act on.
pub fn to_model_messages(session: &StoredSession) -> Vec<anvil_model::Message> {
    let mut out = Vec::new();
    for entry in &session.messages {
        match entry {
            TranscriptEntry::User(t) => {
                if !t.trim().is_empty() {
                    out.push(anvil_model::Message::User(t.clone()));
                }
            }
            TranscriptEntry::Assistant(t) => {
                if !t.trim().is_empty() {
                    out.push(anvil_model::Message::Assistant(anvil_model::AssistantMessage {
                        content: Some(t.clone()),
                        tool_calls: Vec::new(),
                    }));
                }
            }
            TranscriptEntry::ToolResult(r) => {
                out.push(anvil_model::Message::Tool(anvil_model::ToolMessage {
                    tool_call_id: r.tool_call_id.clone(),
                    content: format!("{}\n{}", r.summary, r.output),
                }));
            }
            TranscriptEntry::ToolCall(_) | TranscriptEntry::Error(_) | TranscriptEntry::System(_) => {}
        }
    }
    out
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn save(session: &StoredSession) -> anyhow::Result<PathBuf> {
    if !valid_id(&session.id) {
        anyhow::bail!("refusing to save session with bad id");
    }
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir)?;
    let mut s = session.clone();
    s.updated_at = now_secs();
    if s.messages.len() > MAX_SAVED_MESSAGES {
        let skip = s.messages.len() - MAX_SAVED_MESSAGES;
        s.messages = s.messages.into_iter().skip(skip).collect();
    }
    let path = dir.join(format!("{}.json", s.id));
    std::fs::write(&path, serde_json::to_string_pretty(&s)?)?;
    Ok(path)
}

pub fn load(id: &str) -> anyhow::Result<StoredSession> {
    if !valid_id(id) {
        anyhow::bail!("bad session id");
    }
    let path = sessions_dir().join(format!("{id}.json"));
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

/// Newest first. Corrupt/unreadable files are skipped, never fatal.
pub fn list() -> Vec<SessionMeta> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(sessions_dir()) else {
        return out;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(s): Result<StoredSession, _> = serde_json::from_str(&content) else {
            continue;
        };
        out.push(SessionMeta {
            id: s.id,
            model: s.model,
            messages: s.messages.len(),
            updated_at: s.updated_at,
        });
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.id.cmp(&a.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::TranscriptEntry;
    use std::sync::Mutex;

    /// Tests share one process env; serialize `ANVIL_DATA_DIR` access.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn with_temp_data(f: impl FnOnce()) {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "anvil-sess-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var("ANVIL_DATA_DIR", &dir);
        f();
        std::env::remove_var("ANVIL_DATA_DIR");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn sample(id: &str) -> StoredSession {
        StoredSession {
            version: 1,
            id: id.to_string(),
            model: "m".to_string(),
            base_url: "https://x/v1".to_string(),
            updated_at: 0,
            messages: vec![TranscriptEntry::User("hi".to_string())],
        }
    }

    #[test]
    fn save_load_list_roundtrip() {
        with_temp_data(|| {
            assert!(list().is_empty());
            save(&sample("abc-1")).unwrap();
            let loaded = load("abc-1").unwrap();
            assert_eq!(loaded.messages.len(), 1);
            assert_eq!(loaded.model, "m");
            let metas = list();
            assert_eq!(metas.len(), 1);
            assert_eq!(metas[0].id, "abc-1");
        });
    }

    #[test]
    fn rejects_path_traversal_and_skips_corrupt_files() {
        with_temp_data(|| {
            assert!(load("../evil").is_err());
            assert!(save(&sample("../evil")).is_err());
            std::fs::create_dir_all(sessions_dir()).unwrap();
            std::fs::write(sessions_dir().join("broken.json"), "{not json").unwrap();
            assert!(list().is_empty());
        });
    }
}
