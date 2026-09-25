//! Built-in tools (SPEC §10-11): native `anvil.*` tools with permission
//! categories, workspace sandboxing, and a registry feeding model `ToolSpec`s.

pub mod filesystem;
pub mod git;
pub mod registry;
pub mod search;
pub mod shell;

pub use registry::{ToolContext, ToolOutput, ToolRegistry};

/// Tool permission category (SPEC §12). The agent maps these to the
/// configured allow/ask/deny policy; see `anvil-core` permissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCategory {
    ReadOnly,
    SafeWrite,
    Shell,
    Destructive,
    Network,
}

/// Typed tool errors (SPEC §27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    InvalidArgs(String),
    Io(String),
    Denied(String),
    Sandbox(String),
    Failed(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::InvalidArgs(e) => write!(f, "invalid tool arguments: {e}"),
            ToolError::Io(e) => write!(f, "tool I/O error: {e}"),
            ToolError::Denied(e) => write!(f, "tool denied: {e}"),
            ToolError::Sandbox(e) => write!(f, "path escapes workspace: {e}"),
            ToolError::Failed(e) => write!(f, "tool failed: {e}"),
        }
    }
}

impl std::error::Error for ToolError {}

/// A single executable tool.
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Namespaced name, e.g. `anvil.read_file` (SPEC §16).
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> serde_json::Value;
    fn category(&self) -> ToolCategory;
    /// One-line preview for approval prompts, e.g. `cargo test`.
    fn preview(&self, args: &serde_json::Value) -> String;
    async fn execute(
        &self,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolOutput, ToolError>;
}

/// Resolve `path` (workspace-relative or absolute) to an absolute path
/// inside `workspace`. Rejects escapes, including via symlinks (SPEC §28).
pub fn sandbox_path(workspace: &std::path::Path, path: &str) -> Result<std::path::PathBuf, ToolError> {
    let candidate = if std::path::Path::new(path).is_absolute() {
        std::path::PathBuf::from(path)
    } else {
        workspace.join(path)
    };
    // Canonicalize the workspace root (resolves symlinks like /tmp).
    let ws = workspace.canonicalize().map_err(|e| ToolError::Io(e.to_string()))?;
    // Canonicalize existing prefixes; for not-yet-existing files, walk up
    // to the nearest existing ancestor then re-append the remainder.
    let mut cur = candidate.clone();
    let mut tail = Vec::new();
    loop {
        match cur.canonicalize() {
            Ok(c) => {
                let mut resolved = c;
                tail.reverse();
                for part in tail {
                    resolved.push(part);
                }
                // Lexical normalization only (no symlinks remain below the
                // canonicalized anchor); then enforce containment.
                let final_path = clean_path(&resolved);
                if final_path.starts_with(&ws) {
                    return Ok(final_path);
                }
                return Err(ToolError::Sandbox(path.to_string()));
            }
            Err(_) => {
                if let Some(parent) = cur.parent() {
                    if let Some(name) = cur.file_name() {
                        tail.push(name.to_os_string());
                        cur = parent.to_path_buf();
                        continue;
                    }
                }
                return Err(ToolError::Io(format!("cannot resolve {path}")));
            }
        }
    }
}

/// Lexical path normalization (resolves `.` and `..` without touching fs).
fn clean_path(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn arg_str(args: &serde_json::Value, key: &str) -> Result<String, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing string argument `{key}`")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_rejects_escape() {
        let dir = std::env::temp_dir().join(format!("anvil-sb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(sandbox_path(&dir, "a/b.txt").is_ok());
        assert!(sandbox_path(&dir, "../evil.txt").is_err());
        assert!(sandbox_path(&dir, "/etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_path_resolves_dots() {
        assert_eq!(
            clean_path(std::path::Path::new("/a/b/../c/./d")),
            std::path::PathBuf::from("/a/c/d")
        );
    }
}
