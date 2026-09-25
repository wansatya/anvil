//! The iterative runtime. Owns conversation state, context assembly,
//! tool dispatch, skill lazy-loading, usage accounting, and termination.

use std::sync::Arc;
use std::time::Instant;

use anvil_model::{
    AssistantMessage, ChatRequest, Message, ModelError, ModelProvider, StreamEvent, ToolMessage,
    ToolSpec,
};
use anvil_tools::{ToolContext, ToolRegistry};
use tokio::sync::{mpsc, watch};

use crate::agent::ApprovalGate;
use crate::context::{
    budget::estimate_tokens,
    builder::{CompiledContext, ContextBuilder, ContextInput},
    instructions, relevance, Workspace,
};
use crate::events::{
    AgentEvent, AgentEvent as Ev, ApprovalDecision, ApprovalRequest, ToolCall as EvCall,
    ToolResult as EvResult,
};
use crate::permissions::{PermissionSet, Policy};

/// Typed agent errors (SPEC §27).
#[derive(Debug)]
pub enum AgentError {
    Provider(ModelError),
    Tool(String),
    Cancelled,
    Approval(String),
    MaxIterations(u32),
}

impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentError::Provider(e) => write!(f, "{e}"),
            AgentError::Tool(e) => write!(f, "tool error: {e}"),
            AgentError::Cancelled => write!(f, "cancelled"),
            AgentError::Approval(e) => write!(f, "approval error: {e}"),
            AgentError::MaxIterations(n) => write!(f, "stopped after {n} iterations"),
        }
    }
}

impl std::error::Error for AgentError {}

#[derive(Debug, Clone)]
pub struct AgentOutcome {
    pub final_text: String,
    pub iterations: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub model: String,
    pub max_iterations: u32,
    pub context_max_tokens: usize,
    pub temperature: f32,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { model: String::new(), max_iterations: 50, context_max_tokens: 32000, temperature: 0.2 }
    }
}

pub struct Agent {
    pub config: AgentConfig,
    pub provider: Arc<dyn ModelProvider>,
    pub tools: Arc<ToolRegistry>,
    pub tool_ctx: ToolContext,
    /// Session policy incl. `Always` grants (short std locks, never held
    /// across `.await`).
    pub permissions: std::sync::Mutex<PermissionSet>,
    pub skills: Vec<anvil_skills::Skill>,
    pub workspace: Workspace,
}

impl Agent {
    /// Run one user task to completion. `history` holds prior normalized
    /// conversation and is extended in place (assistant + tool turns), so
    /// follow-up tasks keep full context.
    pub async fn run(
        &self,
        user_request: String,
        history: &mut Vec<Message>,
        events: mpsc::UnboundedSender<Ev>,
        cancel: watch::Receiver<bool>,
        approvals: ApprovalGate,
    ) -> Result<AgentOutcome, AgentError> {
        let started = Instant::now();
        let send = |e: AgentEvent| {
            let _ = events.send(e);
        };
        if *cancel.borrow() {
            send(Ev::Cancelled);
            return Err(AgentError::Cancelled);
        }
        history.push(Message::User(user_request.clone()));
        send(Ev::Thinking);

        // --- static per-task context (SPEC §18 relevance) ---
        let refs = relevance::explicit_refs(&user_request);
        let mut relevant_files = Vec::new();
        for r in refs.iter().take(10) {
            let p = self.workspace.root.join(r);
            if p.is_file() {
                if let Ok(content) = std::fs::read_to_string(&p) {
                    let short: String = content.chars().take(12_000).collect();
                    relevant_files.push((self.workspace.rel(&p), short));
                }
            }
        }
        for p in relevance::recent_files(&self.workspace.root, 5) {
            let rel = self.workspace.rel(&p);
            if relevant_files.iter().any(|(q, _)| q == &rel) {
                continue;
            }
            if relevant_files.len() >= 8 {
                break;
            }
            if let Ok(content) = std::fs::read_to_string(&p) {
                if content.len() > 24_000 || content.as_bytes().iter().take(8000).any(|&b| b == 0) {
                    continue;
                }
                relevant_files.push((rel, content));
            }
        }
        let target = self.workspace.root.clone();
        let instructions = instructions::render(&instructions::load_chain(&self.workspace.root, &target));
        let git_state = crate::context::git::summarize(&self.workspace.root);
        let skill_catalog = anvil_skills::render_catalog(&self.skills);

        let builder = ContextBuilder::new(self.config.context_max_tokens);
        let specs: Vec<ToolSpec> = self.tools.specs();
        let mut active_skill = String::new();
        let mut outcome = AgentOutcome {
            final_text: String::new(),
            iterations: 0,
            tool_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
        };
        let mut current_request = user_request;
        // The request itself is already in `history`; the builder skips
        // empty requests so iteration 1 doesn't duplicate it.
        let mut first_iteration = true;
        let mut cancel = cancel;

        for iteration in 1..=self.config.max_iterations.max(1) {
            if *cancel.borrow() {
                send(Ev::Cancelled);
                return Err(AgentError::Cancelled);
            }
            outcome.iterations = iteration;
            send(Ev::Usage {
                status: status_line(
                    &self.config.model,
                    iteration,
                    self.config.max_iterations.max(1),
                    "asking model…",
                    outcome.input_tokens,
                    outcome.output_tokens,
                    started,
                ),
            });

            let mut system_extra = skill_catalog.clone();
            if !active_skill.is_empty() {
                system_extra.push_str("\n\nActive skill:\n");
                system_extra.push_str(&active_skill);
            }
            let input = ContextInput {
                skill: system_extra,
                instructions: instructions.clone(),
                tool_results: Vec::new(), // live in `history` as Tool messages
                relevant_files: relevant_files.clone(),
                git_state: git_state.clone(),
                history: history.clone(),
            };
            let compiled: CompiledContext = if first_iteration {
                first_iteration = false;
                builder.build("", &input)
            } else {
                builder.build(&current_request, &input)
            };

            let mut req = ChatRequest::new(&self.config.model, compiled.messages);
            req.messages.insert(0, Message::System(compiled.system_prompt));
            req.tools = specs.clone();
            req.temperature = self.config.temperature;

            // --- stream one model turn ---
            // A chat() failure must surface as an event: returning silently
            // would leave the UI on "working…" forever with input blocked.
            let mut stream = match self.provider.chat(req, cancel.clone()).await {
                Ok(s) => s,
                Err(e) => {
                    send(Ev::Error(e.to_string()));
                    return Err(AgentError::Provider(e));
                }
            };
            let mut text = String::new();
            let mut calls = Vec::new();
            let mut stream_failed: Option<ModelError> = None;
            loop {
                tokio::select! {
                    biased;
                    changed = cancel.changed() => {
                        // Err = sender dropped (owner gone): stop either way.
                        if changed.is_err() || *cancel.borrow() {
                            send(Ev::Cancelled);
                            return Err(AgentError::Cancelled);
                        }
                    }
                    item = stream.recv() => {
                        match item {
                            None => break,
                            Some(Err(e)) => { stream_failed = Some(e); break; }
                            Some(Ok(StreamEvent::TextDelta(d))) => {
                                text.push_str(&d);
                                send(Ev::TextDelta(d));
                            }
                            Some(Ok(StreamEvent::ToolCall(tc))) => calls.push(tc),
                            Some(Ok(StreamEvent::Usage { input_tokens, output_tokens })) => {
                                outcome.input_tokens += input_tokens;
                                outcome.output_tokens += output_tokens;
                                send(Ev::Usage { status: status_line(&self.config.model, iteration, self.config.max_iterations.max(1), "streaming…", outcome.input_tokens, outcome.output_tokens, started) });
                            }
                            Some(Ok(StreamEvent::Done)) => break,
                        }
                    }
                }
            }
            if let Some(e) = stream_failed {
                if e == ModelError::Cancelled {
                    send(Ev::Cancelled);
                    return Err(AgentError::Cancelled);
                }
                send(Ev::Error(e.to_string()));
                return Err(AgentError::Provider(e));
            }
            send(Ev::TextDone);
            history.push(Message::Assistant(AssistantMessage { content: if text.is_empty() { None } else { Some(text.clone()) }, tool_calls: calls.clone() }));

            // --- lazy skill load: `use skill <name>` directive ---
            if let Some(name) = detect_skill_request(&text) {
                if let Some(skill) = self.skills.iter().find(|s| s.name == name) {
                    match anvil_skills::load_body(skill) {
                        Ok(body) => {
                            active_skill = body;
                            send(Ev::Note(format!("Loaded skill `{name}`.")));
                            history.push(Message::User(format!(
                                "Skill `{name}` instructions are now active. Continue the task using them."
                            )));
                            current_request = "Continue.".to_string();
                            continue;
                        }
                        Err(e) => {
                            history.push(Message::User(format!("Skill `{name}` failed to load ({e}); continue without it.")));
                            current_request = "Continue.".to_string();
                            continue;
                        }
                    }
                }
            }

            if calls.is_empty() {
                outcome.final_text = text;
                send(Ev::Completed);
                return Ok(outcome);
            }

            // --- execute tool calls ---
            send(Ev::Usage {
                status: status_line(
                    &self.config.model,
                    iteration,
                    self.config.max_iterations.max(1),
                    &format!("running {} tool(s)…", calls.len()),
                    outcome.input_tokens,
                    outcome.output_tokens,
                    started,
                ),
            });
            for tc in calls {
                outcome.tool_calls += 1;
                let tool = self.tools.get(&tc.name);
                let Some(tool) = tool else {
                    let msg = format!("unknown tool `{}`", tc.name);
                    send(Ev::ToolOutput(EvResult {
                        tool_call_id: tc.id.clone(),
                        success: false,
                        summary: "unknown tool".into(),
                        output: msg.clone(),
                    }));
                    history.push(Message::Tool(ToolMessage { tool_call_id: tc.id, content: format!("Error: {msg}") }));
                    continue;
                };
                let policy = self
                    .permissions
                    .lock()
                    .map(|p| p.for_tool(&tc.name, tool.category()))
                    .unwrap_or(Policy::Ask);
                match policy {
                    Policy::Deny => {
                        let msg = format!("Policy denies `{}` in this session.", tc.name);
                        send(Ev::ToolOutput(EvResult {
                            tool_call_id: tc.id.clone(),
                            success: false,
                            summary: "denied by policy".into(),
                            output: msg.clone(),
                        }));
                        history.push(Message::Tool(ToolMessage { tool_call_id: tc.id, content: format!("Error: {msg}") }));
                        continue;
                    }
                    Policy::Ask => {
                        let req = ApprovalRequest {
                            tool_call: EvCall::new(&tc.id, &tc.name, tc.arguments.clone(), &tool.preview(&tc.arguments)),
                            preview: tool.preview(&tc.arguments),
                        };
                        send(Ev::ApprovalRequired(req.clone()));
                        match approvals.ask(req, &mut cancel).await {
                            Ok(ApprovalDecision::AllowOnce) => {}
                            Ok(ApprovalDecision::AllowAlways) => {
                                if let Ok(mut p) = self.permissions.lock() {
                                    p.grant_always(&tc.name);
                                }
                            }
                            Ok(ApprovalDecision::Deny) | Err(_) => {
                                if *cancel.borrow() {
                                    send(Ev::Cancelled);
                                    return Err(AgentError::Cancelled);
                                }
                                let msg = format!("User denied `{}`.", tc.name);
                                send(Ev::ToolOutput(EvResult {
                                    tool_call_id: tc.id.clone(),
                                    success: false,
                                    summary: "denied".into(),
                                    output: msg.clone(),
                                }));
                                history.push(Message::Tool(ToolMessage { tool_call_id: tc.id, content: format!("Error: {msg}") }));
                                continue;
                            }
                        }
                    }
                    Policy::Allow => {}
                }
                send(Ev::ToolStarted(EvCall::new(&tc.id, &tc.name, tc.arguments.clone(), &tool.preview(&tc.arguments))));
                let exec_fut = self.tools.execute(&tc.name, tc.arguments.clone(), self.tool_ctx.clone());
                tokio::pin!(exec_fut);
                let result = loop {
                    tokio::select! {
                        biased;
                        changed = cancel.changed() => {
                            if changed.is_err() || *cancel.borrow() {
                                send(Ev::Cancelled);
                                return Err(AgentError::Cancelled);
                            }
                            // Spurious wake (value unchanged): poll again.
                        }
                        r = &mut exec_fut => break r,
                    }
                };
                match result {
                    Ok(out) => {
                        send(Ev::ToolOutput(EvResult {
                            tool_call_id: tc.id.clone(),
                            success: out.success,
                            summary: out.summary.clone(),
                            output: out.output.clone(),
                        }));
                        history.push(Message::Tool(ToolMessage {
                            tool_call_id: tc.id,
                            content: format!("{}\n{}", out.summary, out.output),
                        }));
                    }
                    Err(e) => {
                        send(Ev::ToolOutput(EvResult {
                            tool_call_id: tc.id.clone(),
                            success: false,
                            summary: "tool error".into(),
                            output: e.to_string(),
                        }));
                        history.push(Message::Tool(ToolMessage {
                            tool_call_id: tc.id,
                            content: format!("Error: {e}"),
                        }));
                    }
                }
            }
            current_request = "Continue with the tool results above.".to_string();

            // Keep history within budget: drop oldest turns, keep the first.
            truncate_history(history, self.config.context_max_tokens);
        }

        send(Ev::Error(format!("Stopped after {} iterations.", self.config.max_iterations)));
        Err(AgentError::MaxIterations(self.config.max_iterations))
    }
}

fn status_line(
    model: &str,
    iteration: u32,
    max_iterations: u32,
    phase: &str,
    input: u64,
    output: u64,
    started: Instant,
) -> String {
    let total = input + output;
    let tokens = if total >= 1000 { format!("{:.1}k", total as f64 / 1000.0) } else { total.to_string() };
    format!(
        "model: {model}  iter: {iteration}/{max_iterations}  {phase}  tokens: {tokens}  latency: {:.1}s",
        started.elapsed().as_secs_f64()
    )
}

/// `use skill <name>` directive (own line, case-insensitive name match).
fn detect_skill_request(text: &str) -> Option<String> {
    for line in text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("use skill").or_else(|| t.strip_prefix("Use skill")) {
            let name = rest.trim().trim_matches([':', '"', '\'']).split_whitespace().next().unwrap_or("");
            let name = name.trim_end_matches(&['.', ',', '!', '?']);
            if !name.is_empty() {
                return Some(name.to_lowercase());
            }
        }
    }
    None
}

fn truncate_history(history: &mut Vec<Message>, max_tokens: usize) {
    let cost = |m: &Message| estimate_tokens(&format!("{m:?}"));
    let mut total: usize = history.iter().map(cost).sum();
    while history.len() > 2 && total > max_tokens {
        history.remove(1); // keep the original request at index 0
        total = history.iter().map(cost).sum();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anvil_model::{MockProvider, RequestedToolCall, ScriptedTurn};
    use anvil_tools::ToolRegistry;

    fn test_agent(provider: MockProvider) -> Agent {
        let dir = std::env::temp_dir().join(format!("anvil-agent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Agent {
            config: AgentConfig { model: "mock".into(), max_iterations: 5, context_max_tokens: 8000, temperature: 0.0 },
            provider: Arc::new(provider),
            tools: Arc::new(ToolRegistry::new()),
            tool_ctx: ToolContext::new(&dir),
            permissions: std::sync::Mutex::new(PermissionSet::defaults()),
            skills: vec![],
            workspace: Workspace { root: dir },
        }
    }

    async fn drain(rx: &mut mpsc::UnboundedReceiver<AgentEvent>, ) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            out.push(e);
        }
        out
    }

    #[tokio::test]
    async fn final_text_completes_without_tools() {
        let agent = test_agent(MockProvider::new(vec![ScriptedTurn::text(&["done!"])]));
        let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
        let (_c_tx, c_rx) = watch::channel(false);
        let (ap_tx, _ap_rx) = mpsc::unbounded_channel();
        let outcome = agent.run("hi".into(), &mut vec![], ev_tx, c_rx, ApprovalGate::new(ap_tx)).await.unwrap();
        assert_eq!(outcome.final_text, "done!");
        let evs = drain(&mut ev_rx).await;
        assert!(evs.contains(&Ev::Thinking));
        assert!(evs.contains(&Ev::Completed));
        assert!(evs.iter().any(|e| matches!(e, Ev::TextDelta(_))));
    }

    #[tokio::test]
    async fn history_accumulates_across_turns() {
        let agent = test_agent(MockProvider::new(vec![ScriptedTurn::text(&["first"])]));
        let (ev_tx, _ev_rx) = mpsc::unbounded_channel();
        let (_c_tx, c_rx) = watch::channel(false);
        let (ap_tx, _ap_rx) = mpsc::unbounded_channel();
        let mut history = Vec::new();
        agent.run("hi".into(), &mut history, ev_tx, c_rx.clone(), ApprovalGate::new(ap_tx)).await.unwrap();
        assert_eq!(history.len(), 2);
        assert!(matches!(history[0], Message::User(_)));
        assert!(matches!(history[1], Message::Assistant(_)));
        // Second turn builds on the first: history keeps growing.
        let (ev_tx2, _ev_rx2) = mpsc::unbounded_channel();
        let (ap_tx2, _ap_rx2) = mpsc::unbounded_channel();
        agent.run("again".into(), &mut history, ev_tx2, c_rx, ApprovalGate::new(ap_tx2)).await.unwrap();
        assert_eq!(history.len(), 4);
    }

    #[tokio::test]
    async fn unknown_tool_feeds_back_and_continues() {
        let agent = test_agent(MockProvider::new(vec![
            ScriptedTurn {
                text: vec!["trying".into()],
                tool_calls: vec![RequestedToolCall { id: "1".into(), name: "nope".into(), arguments: serde_json::json!({}) }],
            },
            ScriptedTurn::text(&["gave up"]),
        ]));
        let (ev_tx, _ev_rx) = mpsc::unbounded_channel();
        let (_c_tx, c_rx) = watch::channel(false);
        let (ap_tx, _ap_rx) = mpsc::unbounded_channel();
        let outcome = agent.run("hi".into(), &mut vec![], ev_tx, c_rx, ApprovalGate::new(ap_tx)).await.unwrap();
        assert_eq!(outcome.final_text, "gave up");
        assert_eq!(outcome.tool_calls, 1);
    }

    #[tokio::test]
    async fn cancel_before_start_aborts() {
        let agent = test_agent(MockProvider::new(vec![ScriptedTurn::text(&["x"])]));
        let (ev_tx, _ev_rx) = mpsc::unbounded_channel();
        let (_c_tx, c_rx) = watch::channel(true);
        let (ap_tx, _ap_rx) = mpsc::unbounded_channel();
        let err = agent.run("hi".into(), &mut vec![], ev_tx, c_rx, ApprovalGate::new(ap_tx)).await.unwrap_err();
        assert!(matches!(err, AgentError::Cancelled));
    }

    #[tokio::test]
    async fn deny_policy_skips_execution() {
        let mut agent = test_agent(MockProvider::new(vec![
            ScriptedTurn {
                text: vec![],
                tool_calls: vec![RequestedToolCall { id: "1".into(), name: "anvil.shell".into(), arguments: serde_json::json!({"command": "echo hi"}) }],
            },
            ScriptedTurn::text(&["done"]),
        ]));
        let mut reg = ToolRegistry::new();
        reg.register(anvil_tools::shell::Shell);
        agent.tools = Arc::new(reg);
        {
            let mut p = agent.permissions.lock().unwrap();
            p.shell = Policy::Deny;
        }
        let (ev_tx, mut ev_rx) = mpsc::unbounded_channel();
        let (_c_tx, c_rx) = watch::channel(false);
        let (ap_tx, _ap_rx) = mpsc::unbounded_channel();
        let outcome = agent.run("hi".into(), &mut vec![], ev_tx, c_rx, ApprovalGate::new(ap_tx)).await.unwrap();
        assert_eq!(outcome.final_text, "done");
        let evs = drain(&mut ev_rx).await;
        assert!(evs.iter().any(|e| matches!(e, Ev::ToolOutput(r) if r.summary == "denied by policy")));
    }

    #[test]
    fn skill_directive_detected() {
        assert_eq!(detect_skill_request("Let me think.\nuse skill cpp-review"), Some("cpp-review".into()));
        assert_eq!(detect_skill_request("nothing"), None);
    }
}
