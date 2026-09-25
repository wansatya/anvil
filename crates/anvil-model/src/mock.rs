//! Scripted provider for tests: no network, no API key (SPEC §33).

use tokio::sync::{mpsc, watch};

use crate::provider::{ChatRequest, ChatStream, ModelError, ModelProvider, StreamEvent};

/// A scripted turn: text chunks then optional tool calls.
#[derive(Debug, Clone)]
pub struct ScriptedTurn {
    pub text: Vec<String>,
    pub tool_calls: Vec<crate::types::RequestedToolCall>,
}

impl ScriptedTurn {
    pub fn text(words: &[&str]) -> Self {
        Self {
            text: words.iter().map(|w| w.to_string()).collect(),
            tool_calls: Vec::new(),
        }
    }
}

/// Pops one scripted turn per `chat()` call; repeats the last when exhausted.
pub struct MockProvider {
    pub turns: std::sync::Mutex<Vec<ScriptedTurn>>,
}

impl MockProvider {
    pub fn new(turns: Vec<ScriptedTurn>) -> Self {
        Self { turns: std::sync::Mutex::new(turns) }
    }
}

#[async_trait::async_trait]
impl ModelProvider for MockProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn chat(
        &self,
        _request: ChatRequest,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<ChatStream, ModelError> {
        let turn = {
            let mut turns = self.turns.lock().unwrap();
            if turns.len() > 1 {
                turns.remove(0)
            } else {
                turns.first().cloned().unwrap_or(ScriptedTurn { text: vec![], tool_calls: vec![] })
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            for chunk in turn.text {
                if *cancel.borrow() {
                    let _ = tx.send(Err(ModelError::Cancelled));
                    return;
                }
                if tx.send(Ok(StreamEvent::TextDelta(chunk))).is_err() {
                    return;
                }
                tokio::task::yield_now().await;
            }
            for tc in turn.tool_calls {
                let _ = tx.send(Ok(StreamEvent::ToolCall(tc)));
            }
            let _ = tx.send(Ok(StreamEvent::Done));
            let _ = cancel.changed().await;
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_replays_turns() {
        let p = MockProvider::new(vec![
            ScriptedTurn::text(&["hi ", "there"]),
            ScriptedTurn::text(&["again"]),
        ]);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let mut s = p.chat(ChatRequest::new("m", vec![]), cancel_rx).await.unwrap();
        assert_eq!(s.recv().await.unwrap().unwrap(), StreamEvent::TextDelta("hi ".into()));
        assert_eq!(s.recv().await.unwrap().unwrap(), StreamEvent::TextDelta("there".into()));
        assert_eq!(s.recv().await.unwrap().unwrap(), StreamEvent::Done);
    }
}
