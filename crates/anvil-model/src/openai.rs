//! OpenAI-compatible streaming provider (SPEC §8).
//! `POST {base_url}/chat/completions` with `stream: true` (SSE).
//! Used for OpenCode Zen and any OpenAI-compatible endpoint.

use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::provider::{ChatRequest, ChatStream, ModelError, ModelProvider, StreamEvent};
use crate::types::{AssistantMessage, Message, RequestedToolCall, ToolMessage};

pub struct OpenAiCompatible {
    pub base_url: String,
    pub api_key: String,
    client: reqwest::Client,
    send_timeout: std::time::Duration,
    chunk_idle_timeout: std::time::Duration,
}

/// Fail the request if the server takes longer than this to answer.
const DEFAULT_SEND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Fail the stream if no bytes arrive for this long (stall detection).
const DEFAULT_CHUNK_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

impl OpenAiCompatible {
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            send_timeout: DEFAULT_SEND_TIMEOUT,
            chunk_idle_timeout: DEFAULT_CHUNK_IDLE_TIMEOUT,
        }
    }

    /// Override timeouts (used by tests; production uses the defaults).
    pub fn with_timeouts(mut self, send: std::time::Duration, chunk_idle: std::time::Duration) -> Self {
        self.send_timeout = send;
        self.chunk_idle_timeout = chunk_idle;
        self
    }

    fn messages_to_wire(messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|m| match m {
                Message::System(t) => serde_json::json!({"role": "system", "content": t}),
                Message::User(t) => serde_json::json!({"role": "user", "content": t}),
                Message::Assistant(a) => {
                    let AssistantMessage { content, tool_calls } = a;
                    let mut obj = serde_json::json!({"role": "assistant"});
                    if let Some(c) = content {
                        obj["content"] = serde_json::Value::String(c.clone());
                    } else {
                        obj["content"] = serde_json::Value::Null;
                    }
                    if !tool_calls.is_empty() {
                        obj["tool_calls"] = serde_json::Value::Array(
                            tool_calls
                                .iter()
                                .map(|tc| {
                                    serde_json::json!({
                                        "id": tc.id,
                                        "type": "function",
                                        "function": {
                                            "name": tc.name,
                                            "arguments": tc.arguments.to_string(),
                                        }
                                    })
                                })
                                .collect(),
                        );
                    }
                    obj
                }
                Message::Tool(ToolMessage { tool_call_id, content }) => {
                    serde_json::json!({"role": "tool", "tool_call_id": tool_call_id, "content": content})
                }
            })
            .collect()
    }

    fn tools_to_wire(tools: &[crate::types::ToolSpec]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl ModelProvider for OpenAiCompatible {
    fn name(&self) -> &str {
        "openai-compatible"
    }

    async fn chat(
        &self,
        request: ChatRequest,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<ChatStream, ModelError> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = serde_json::json!({
            "model": request.model,
            "messages": Self::messages_to_wire(&request.messages),
            "tools": Self::tools_to_wire(&request.tools),
            "temperature": request.temperature,
            "max_tokens": request.max_tokens,
            "stream": true,
        });
        let mut req = self.client.post(&url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = tokio::time::timeout(self.send_timeout, req.send())
            .await
            .map_err(|_| {
                ModelError::Transport(format!(
                    "provider did not answer within {}s — check base URL / network",
                    self.send_timeout.as_secs()
                ))
            })?
            .map_err(|e| {
                if e.is_connect() || e.is_timeout() {
                    ModelError::Transport(e.to_string())
                } else if e.is_body() || e.is_decode() {
                    ModelError::Parse(e.to_string())
                } else {
                    ModelError::Transport(e.to_string())
                }
            })?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(ModelError::Auth(format!(
                "HTTP {status} — check the API key (and that it belongs to this provider)"
            )));
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ModelError::RateLimited(format!("HTTP {status}")));
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            let text = resp.text().await.unwrap_or_default();
            let short: String = text.chars().take(300).collect();
            return Err(ModelError::BadRequest(format!(
                "HTTP 404: model or endpoint not found at this provider — check the model id and base URL. Server said: {short}"
            )));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            let short: String = text.chars().take(300).collect();
            return Err(ModelError::BadRequest(format!("HTTP {status}: {short}")));
        }

        let (tx, rx) = mpsc::unbounded_channel();
        let chunk_idle = self.chunk_idle_timeout;
        tokio::spawn(async move {
            let mut stream = resp.bytes_stream();
            let mut buf = Vec::<u8>::new();
            // Accumulated function-call fragments per stream index.
            let mut calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
            let mut cancelled = false;
            // Once the sender is gone nobody can cancel anymore: stop
            // polling it (it would resolve instantly forever) and just
            // finish the stream.
            let mut cancel_gone = false;
            loop {
                if *cancel.borrow() {
                    cancelled = true;
                    break;
                }
                tokio::select! {
                    biased;
                    res = cancel.changed(), if !cancel_gone => {
                        match res {
                            Err(_) => cancel_gone = true,
                            Ok(()) => if *cancel.borrow() { cancelled = true; break; },
                        }
                    }
                    chunk = tokio::time::timeout(chunk_idle, stream.next()) => {
                        let chunk = match chunk {
                            Ok(c) => c,
                            Err(_) => {
                                let _ = tx.send(Err(ModelError::Transport(format!(
                                    "no data for {}s — the provider stopped responding mid-stream",
                                    chunk_idle.as_secs()
                                ))));
                                return;
                            }
                        };
                        match chunk {
                            Some(Ok(bytes)) => {
                                buf.extend_from_slice(&bytes);
                                drain_sse(&mut buf, &tx, &mut calls);
                            }
                            Some(Err(e)) => {
                                let _ = tx.send(Err(ModelError::Transport(e.to_string())));
                                return;
                            }
                            None => break,
                        }
                    }
                }
            }
            if cancelled {
                let _ = tx.send(Err(ModelError::Cancelled));
                return;
            }
            // Flush any trailing buffered line, then emit completed calls.
            drain_sse_final(&mut buf, &tx, &mut calls);
            for (id, name, args) in calls {
                let arguments = serde_json::from_str(&args).unwrap_or(serde_json::Value::Null);
                let _ = tx.send(Ok(StreamEvent::ToolCall(RequestedToolCall { id, name, arguments })));
            }
            let _ = tx.send(Ok(StreamEvent::Done));
        });
        Ok(rx)
    }
}

/// Pull complete `data:` lines out of `buf`, emitting text deltas and
/// accumulating tool-call fragments into `calls`.
fn drain_sse(
    buf: &mut Vec<u8>,
    tx: &mpsc::UnboundedSender<Result<StreamEvent, ModelError>>,
    calls: &mut Vec<(String, String, String)>,
) {
    drain_sse_inner(buf, tx, calls, false);
}

fn drain_sse_final(
    buf: &mut Vec<u8>,
    tx: &mpsc::UnboundedSender<Result<StreamEvent, ModelError>>,
    calls: &mut Vec<(String, String, String)>,
) {
    drain_sse_inner(buf, tx, calls, true);
}

fn drain_sse_inner(
    buf: &mut Vec<u8>,
    tx: &mpsc::UnboundedSender<Result<StreamEvent, ModelError>>,
    calls: &mut Vec<(String, String, String)>,
    final_flush: bool,
) {
    loop {
        let end = buf.iter().position(|&b| b == b'\n');
        let Some(i) = end else {
            if final_flush && !buf.is_empty() {
                let line = std::mem::take(buf);
                handle_sse_line(&line, tx, calls);
            }
            break;
        };
        let mut line: Vec<u8> = buf.drain(..=i).collect();
        while line.last().is_some_and(|&b| b == b'\n' || b == b'\r') {
            line.pop();
        }
        if line.is_empty() {
            continue;
        }
        handle_sse_line(&line, tx, calls);
    }
}

fn handle_sse_line(
    line: &[u8],
    tx: &mpsc::UnboundedSender<Result<StreamEvent, ModelError>>,
    calls: &mut Vec<(String, String, String)>,
) {
    let line = line.strip_prefix(b"data:").unwrap_or(line);
    let line = strip_spaces(line);
    if line.is_empty() || line == b"[DONE]" {
        return;
    }
    let v: serde_json::Value = match serde_json::from_slice(line) {
        Ok(v) => v,
        Err(e) => {
            let _ = tx.send(Err(ModelError::Parse(format!("bad SSE chunk: {e}"))));
            return;
        }
    };
    if let Some(u) = v.get("usage") {
        let input = u.get("prompt_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
        let output = u.get("completion_tokens").and_then(|n| n.as_u64()).unwrap_or(0);
        let _ = tx.send(Ok(StreamEvent::Usage { input_tokens: input, output_tokens: output }));
    }
    let Some(delta) = v
        .pointer("/choices/0/delta")
        .or_else(|| v.pointer("/choices/0/message"))
    else {
        // Error payloads: { "error": { "message": ... } }
        if let Some(msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            let _ = tx.send(Err(ModelError::BadRequest(msg.to_string())));
        }
        return;
    };
    if let Some(text) = delta.get("content").and_then(|c| c.as_str()) {
        if !text.is_empty() {
            let _ = tx.send(Ok(StreamEvent::TextDelta(text.to_string())));
        }
    }
    let tool_calls = delta.get("tool_calls").and_then(|t| t.as_array()).cloned().unwrap_or_default();
    for tc in tool_calls {
        let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        while calls.len() <= idx {
            calls.push((String::new(), String::new(), String::new()));
        }
        if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
            if !id.is_empty() {
                calls[idx].0 = id.to_string();
            }
        }
        if let Some(fun) = tc.get("function") {
            if let Some(name) = fun.get("name").and_then(|n| n.as_str()) {
                if !name.is_empty() {
                    calls[idx].1 = name.to_string();
                }
            }
            if let Some(args) = fun.get("arguments").and_then(|a| a.as_str()) {
                calls[idx].2.push_str(args);
            }
        }
    }
}

fn strip_spaces(mut b: &[u8]) -> &[u8] {
    while let Some((&first, rest)) = b.split_first() {
        if first == b' ' || first == b'\t' {
            b = rest;
        } else {
            break;
        }
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::watch;

    /// Stub server modes: full raw response, 200-headers-then-stall,
    /// or hold the connection before responding at all.
    enum Stub {
        Respond(&'static str),
        StallAfterHeaders,
        Hang,
    }

    async fn stub_server(mode: Stub) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            // Read request headers + body.
            let mut buf = vec![0u8; 8192];
            use tokio::io::AsyncReadExt;
            let mut head = Vec::new();
            let mut content_len = 0usize;
            loop {
                let Ok(n) = sock.read(&mut buf).await else { return };
                if n == 0 {
                    return;
                }
                head.extend_from_slice(&buf[..n]);
                if let Some(i) = find_headers_end(&head) {
                    let headers = String::from_utf8_lossy(&head[..i]).to_string();
                    for line in headers.lines() {
                        if let Some(v) = line.strip_prefix("content-length:")
                            .or_else(|| line.strip_prefix("Content-Length:"))
                        {
                            content_len = v.trim().parse().unwrap_or(0);
                        }
                    }
                    let body_read = head.len() - i;
                    let mut rest = vec![0u8; content_len.saturating_sub(body_read)];
                    let mut pos = 0;
                    while pos < rest.len() {
                        let Ok(n) = sock.read(&mut rest[pos..]).await else { return };
                        if n == 0 {
                            return;
                        }
                        pos += n;
                    }
                    break;
                }
            }
            use tokio::io::AsyncWriteExt;
            match mode {
                Stub::Respond(raw) => {
                    let _ = sock.write_all(raw.as_bytes()).await;
                }
                Stub::StallAfterHeaders => {
                    let _ = sock
                        .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
                        .await;
                    std::future::pending::<()>().await;
                }
                Stub::Hang => {
                    // Hold forever (client must time out, not hang).
                    std::future::pending::<()>().await;
                }
            }
        });
        format!("http://{addr}")
    }

    fn find_headers_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
    }

    fn test_request() -> (ChatRequest, watch::Receiver<bool>) {
        let (_tx, rx) = watch::channel(false);
        (ChatRequest::new("test-model", vec![Message::User("hi".into())]), rx)
    }

    #[tokio::test]
    async fn not_found_maps_to_actionable_error() {
        let base = stub_server(Stub::Respond(
            "HTTP/1.1 404 Not Found\r\ncontent-length: 16\r\nconnection: close\r\n\r\n{\"error\":\"nope\"}",
        ))
        .await;
        let p = OpenAiCompatible::new(&base, "k");
        let (req, rx) = test_request();
        let err = p.chat(req, rx).await.unwrap_err();
        assert!(matches!(err, ModelError::BadRequest(_)));
        let msg = err.to_string();
        assert!(msg.contains("404"), "{msg}");
        assert!(msg.contains("model id"), "{msg}");
    }

    #[tokio::test]
    async fn hanging_server_times_out_on_send() {
        let base = stub_server(Stub::Hang).await;
        let p = OpenAiCompatible::new(&base, "k").with_timeouts(
            std::time::Duration::from_millis(300),
            std::time::Duration::from_secs(5),
        );
        let (req, rx) = test_request();
        let err = p.chat(req, rx).await.unwrap_err();
        assert!(matches!(err, ModelError::Transport(_)), "{err:?}");
    }

    #[tokio::test]
    async fn stalled_stream_times_out_instead_of_hanging() {
        let base = stub_server(Stub::StallAfterHeaders).await;
        let p = OpenAiCompatible::new(&base, "k").with_timeouts(
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(300),
        );
        let (req, rx) = test_request();
        let mut stream = p.chat(req, rx).await.expect("headers arrive");
        let err = stream.recv().await.expect("stream ends").unwrap_err();
        assert!(matches!(err, ModelError::Transport(_)), "{err:?}");
        assert!(err.to_string().contains("stopped responding"), "{err}");
    }

    #[tokio::test]
    async fn happy_path_streams_text_end_to_end() {
        let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]\n";
        let raw = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            sse.len(),
            sse
        );
        let raw: &'static str = Box::leak(raw.into_boxed_str());
        let base = stub_server(Stub::Respond(raw)).await;
        let p = OpenAiCompatible::new(&base, "k");
        let (req, rx) = test_request();
        let mut stream = p.chat(req, rx).await.unwrap();
        assert_eq!(
            stream.recv().await.unwrap().unwrap(),
            StreamEvent::TextDelta("hello".into())
        );
        assert_eq!(stream.recv().await.unwrap().unwrap(), StreamEvent::Done);
    }

    #[test]
    fn wire_messages_map_roles() {
        let msgs = vec![
            Message::System("sys".into()),
            Message::User("hi".into()),
            Message::Assistant(AssistantMessage { content: None, tool_calls: vec![] }),
            Message::Tool(ToolMessage { tool_call_id: "1".into(), content: "ok".into() }),
        ];
        let wire = OpenAiCompatible::messages_to_wire(&msgs);
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[1]["role"], "user");
        assert_eq!(wire[2]["role"], "assistant");
        assert_eq!(wire[3]["role"], "tool");
        assert_eq!(wire[3]["tool_call_id"], "1");
    }

    #[test]
    fn sse_text_and_fragmented_tool_call() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut calls = Vec::new();
        let mut buf = b"data: {\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"read\", \"arguments\":\"{\\\"pa\"}}]}}]}\n".to_vec();
        drain_sse(&mut buf, &tx, &mut calls);
        assert!(buf.is_empty());
        assert_eq!(rx.try_recv().unwrap().unwrap(), StreamEvent::TextDelta("Hel".into()));
        assert!(rx.try_recv().is_err()); // tool call not yet complete
        let mut buf2 = b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":1}\"}}]}}]}\n".to_vec();
        drain_sse(&mut buf2, &tx, &mut calls);
        assert_eq!(calls[0].0, "c1");
        assert_eq!(calls[0].1, "read");
        assert_eq!(calls[0].2, "{\"path\":1}");
    }

    #[test]
    fn sse_done_and_usage() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut calls = Vec::new();
        let mut buf = b"data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\ndata: [DONE]\n".to_vec();
        drain_sse(&mut buf, &tx, &mut calls);
        assert_eq!(
            rx.try_recv().unwrap().unwrap(),
            StreamEvent::Usage { input_tokens: 10, output_tokens: 5 }
        );
        assert!(rx.try_recv().is_err());
    }
}
