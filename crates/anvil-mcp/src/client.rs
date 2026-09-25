//! Minimal MCP stdio transport: newline-delimited JSON-RPC 2.0.
//! Speaks `initialize`, `tools/list`, `tools/call` (protocol 2024-11-05).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex};

/// Typed MCP errors (SPEC §27).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpError {
    Spawn(String),
    Io(String),
    Protocol(String),
    Timeout,
    ToolFailed(String),
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Spawn(e) => write!(f, "MCP spawn failed: {e}"),
            McpError::Io(e) => write!(f, "MCP I/O error: {e}"),
            McpError::Protocol(e) => write!(f, "MCP protocol error: {e}"),
            McpError::Timeout => write!(f, "MCP request timed out"),
            McpError::ToolFailed(e) => write!(f, "MCP tool failed: {e}"),
        }
    }
}

impl std::error::Error for McpError {}

#[derive(Debug, Clone)]
pub struct McpConfig {
    pub command: String,
    pub args: Vec<String>,
}

impl McpConfig {
    pub fn new(command: &str, args: Vec<String>) -> Self {
        Self { command: command.to_string(), args }
    }
}

#[derive(Debug, Clone)]
pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

struct Pending {
    next_id: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>,
}

/// A running MCP server connection. Cloneable; kill-on-drop via the owned
/// child handle (kept alive by the reader task + explicit shutdown).
#[derive(Clone)]
pub struct McpClient {
    stdin: Arc<Mutex<tokio::process::ChildStdin>>,
    pending: Arc<Pending>,
    _child: Arc<Mutex<Option<tokio::process::Child>>>,
}

impl McpClient {
    /// Spawn the server and run `initialize` + `notifications/initialized`.
    pub async fn spawn(config: &McpConfig) -> Result<Self, McpError> {
        let mut child = tokio::process::Command::new(&config.command)
            .args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| McpError::Spawn(format!("{} {}: {e}", config.command, config.args.join(" "))))?;
        let stdin = child.stdin.take().ok_or_else(|| McpError::Spawn("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| McpError::Spawn("no stdout".into()))?;
        let client = Self {
            stdin: Arc::new(Mutex::new(stdin)),
            pending: Arc::new(Pending { next_id: AtomicU64::new(1), waiting: Mutex::new(HashMap::new()) }),
            _child: Arc::new(Mutex::new(Some(child))),
        };
        // Reader task: route responses to waiters, ignore notifications.
        let reader_client = client.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                reader_client.route(&line).await;
            }
            // EOF: fail all waiters.
            let mut waiting = reader_client.pending.waiting.lock().await;
            for (_, tx) in waiting.drain() {
                let _ = tx.send(serde_json::Value::Null);
            }
        });
        let init = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "anvil", "version": env!("CARGO_PKG_VERSION")},
        });
        client.request("initialize", init).await?;
        client.notify("notifications/initialized", serde_json::json!({})).await?;
        Ok(client)
    }

    async fn route(&self, line: &str) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return };
        // Responses carry `id`; notifications don't.
        let Some(id) = v.get("id").and_then(|i| i.as_u64()) else { return };
        let mut waiting = self.pending.waiting.lock().await;
        if let Some(tx) = waiting.remove(&id) {
            let _ = tx.send(v);
        }
    }

    async fn send_raw(&self, payload: &serde_json::Value) -> Result<(), McpError> {
        let mut text = serde_json::to_string(payload).map_err(|e| McpError::Protocol(e.to_string()))?;
        text.push('\n');
        self.stdin.lock().await.write_all(text.as_bytes()).await.map_err(|e| McpError::Io(e.to_string()))?;
        Ok(())
    }

    async fn notify(&self, method: &str, params: serde_json::Value) -> Result<(), McpError> {
        self.send_raw(&serde_json::json!({"jsonrpc": "2.0", "method": method, "params": params})).await
    }

    async fn request(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value, McpError> {
        let id = self.pending.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.waiting.lock().await.insert(id, tx);
        let send_result = self
            .send_raw(&serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        if let Err(e) = send_result {
            self.pending.waiting.lock().await.remove(&id);
            return Err(e);
        }
        let resp = tokio::time::timeout(std::time::Duration::from_secs(60), rx)
            .await
            .map_err(|_| McpError::Timeout)?
            .map_err(|_| McpError::Io("response channel closed".into()))?;
        if let Some(err) = resp.get("error") {
            return Err(McpError::Protocol(format!("server error: {err}")));
        }
        Ok(resp.get("result").cloned().unwrap_or(serde_json::Value::Null))
    }

    /// `tools/list` → normalized definitions.
    pub async fn list_tools(&self) -> Result<Vec<McpToolDef>, McpError> {
        let result = self.request("tools/list", serde_json::json!({})).await?;
        let mut out = Vec::new();
        for t in result.get("tools").and_then(|t| t.as_array()).cloned().unwrap_or_default() {
            out.push(McpToolDef {
                name: t.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string(),
                description: t.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
                input_schema: t.get("inputSchema").cloned().unwrap_or(serde_json::json!({"type": "object"})),
            });
        }
        Ok(out)
    }

    /// `tools/call` → concatenated text content.
    pub async fn call_tool(&self, name: &str, arguments: serde_json::Value) -> Result<String, McpError> {
        let result = self
            .request("tools/call", serde_json::json!({"name": name, "arguments": arguments}))
            .await?;
        if result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false) {
            let text = flatten_content(&result);
            return Err(McpError::ToolFailed(if text.is_empty() { "unknown error".into() } else { text }));
        }
        Ok(flatten_content(&result))
    }
}

fn flatten_content(result: &serde_json::Value) -> String {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake server speaking enough MCP to test against (python3 or sh?).
    /// Uses `cat`-style echo? No — needs real protocol, so emulate with a
    /// tiny inline responder via `sh` + heredoc-less printf loop is fragile.
    /// Instead test request/response routing with a FIFO-less echo server:
    /// python3 may not exist; test only pure helpers + error paths.
    #[test]
    fn flatten_joins_text_blocks() {
        let v = serde_json::json!({"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}, {"type": "image", "data": "x"}]});
        assert_eq!(flatten_content(&v), "a\nb");
    }

    #[tokio::test]
    async fn spawn_missing_binary_errors() {
        let res = McpClient::spawn(&McpConfig::new("anvil-definitely-missing-binary-xyz", vec![])).await;
        assert!(matches!(res, Err(McpError::Spawn(_))));
    }

    /// Full handshake + tools/list + tools/call against a fake server.
    #[cfg(unix)]
    #[tokio::test]
    async fn protocol_roundtrip_with_fake_server() {
        let script = r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id": *\([0-9]*\).*/\1/p');
  method=$(printf '%s' "$line" | sed -n 's/.*"method": *"\([^"]*\)".*/\1/p');
  case "$method" in
    initialize) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2024-11-05\"}}" ;;
    tools/list) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"tools\":[{\"name\":\"echo\",\"description\":\"echo it\",\"inputSchema\":{\"type\":\"object\"}}]}}" ;;
    tools/call) echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"hello\"}]}}" ;;
  esac;
done"#;
        let client = McpClient::spawn(&McpConfig::new("sh", vec!["-c".into(), script.into()]))
            .await
            .expect("fake server spawns");
        let tools = client.list_tools().await.expect("tools/list works");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(client.call_tool("echo", serde_json::json!({})).await.unwrap(), "hello");
    }
}
