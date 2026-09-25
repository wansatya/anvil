//! Context assembly with token budget (SPEC §17).
//! Priority: system > user request > instructions > skill > tool results
//! > source files > git state > older conversation.

use anvil_model::{AssistantMessage, Message, ToolMessage};

use crate::context::budget::{estimate_tokens, fit_budget};

/// Base system prompt: identity + tool-use contract.
pub const BASE_SYSTEM_PROMPT: &str = "\
You are Anvil, a terminal-native engineering agent. You act on the user's \
project through tools: inspect first, then plan, then change.

Rules:
- Prefer read-only tools (read_file, list_files, search, git_status, git_diff) before editing.
- To act, emit tool calls; never claim you ran something you did not call.
- Keep responses concise. Explain the plan briefly before destructive steps.
- Tool arguments must be exact JSON matching each tool's schema.\
";

/// Everything the builder may draw from.
#[derive(Debug, Clone, Default)]
pub struct ContextInput {
    /// Active skill content (already selected + loaded).
    pub skill: String,
    /// Relevant project instruction block (rendered ANVIL.md chain).
    pub instructions: String,
    /// Recent tool results as (label, output).
    pub tool_results: Vec<(String, String)>,
    /// Relevant source files as (path, content).
    pub relevant_files: Vec<(String, String)>,
    /// Git state summary.
    pub git_state: String,
    /// Older conversation (normalized model messages).
    pub history: Vec<Message>,
}

#[derive(Debug, Clone)]
pub struct CompiledContext {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub estimated_tokens: usize,
}

pub struct ContextBuilder {
    pub max_tokens: usize,
}

impl ContextBuilder {
    /// `max_tokens` is honored as-is (even tiny, for tests); callers should
    /// enforce a sane floor (e.g. ≥4096). System + latest request are always
    /// included even if they alone exceed the budget.
    pub fn new(max_tokens: usize) -> Self {
        Self { max_tokens }
    }

    /// Compile `user_request` + `input` into a budgeted context.
    pub fn build(&self, user_request: &str, input: &ContextInput) -> CompiledContext {
        let mut system = String::from(BASE_SYSTEM_PROMPT);
        if !input.instructions.trim().is_empty() {
            system.push_str("\n\n");
            system.push_str(input.instructions.trim());
        }
        if !input.skill.trim().is_empty() {
            system.push_str("\n\nActive skill:\n");
            system.push_str(input.skill.trim());
        }
        let system_cost = estimate_tokens(&system);

        // Candidate content blocks in priority order (after request).
        let mut blocks: Vec<(Block, usize)> = Vec::new();
        for (label, output) in &input.tool_results {
            let text = format!("Tool result [{label}]:\n{output}");
            blocks.push((Block::ToolResult(label.clone(), output.clone()), estimate_tokens(&text)));
        }
        for (path, content) in &input.relevant_files {
            let text = format!("File {path}:\n{content}");
            blocks.push((Block::File(path.clone(), content.clone()), estimate_tokens(&text)));
        }
        if !input.git_state.trim().is_empty() {
            blocks.push((
                Block::Git(input.git_state.clone()),
                estimate_tokens(&input.git_state),
            ));
        }
        // History is lowest priority and dropped oldest-first.
        let mut history: Vec<(Message, usize)> = input
            .history
            .iter()
            .map(|m| {
                let cost = estimate_tokens(&render_message(m));
                (m.clone(), cost)
            })
            .collect();
        // Drop oldest history first when over budget: reverse, fit, reverse back.
        history.reverse();

        let request_cost = estimate_tokens(user_request);
        let mut budget = self.max_tokens.saturating_sub(system_cost + request_cost);

        let (kept_blocks, _) = fit_budget(blocks, budget);
        budget = budget.saturating_sub(kept_blocks.iter().map(block_cost).sum::<usize>());
        let (kept_hist_rev, _) = fit_budget(history, budget);
        let mut kept_history = kept_hist_rev;
        kept_history.reverse();

        let mut messages: Vec<Message> = kept_history;
        for b in kept_blocks {
            match b {
                Block::ToolResult(label, output) => {
                    messages.push(Message::Tool(ToolMessage {
                        tool_call_id: label,
                        content: output,
                    }));
                }
                Block::File(path, content) => {
                    messages.push(Message::User(format!("File {path}:\n{content}")));
                }
                Block::Git(g) => {
                    messages.push(Message::User(format!("Repository state:\n{g}")));
                }
            }
        }
        messages.push(Message::User(user_request.to_string()));

        let estimated_tokens = system_cost
            + messages.iter().map(render_message).map(|s| estimate_tokens(&s)).sum::<usize>();
        CompiledContext { system_prompt: system, messages, estimated_tokens }
    }
}

#[derive(Debug, Clone)]
enum Block {
    ToolResult(String, String),
    File(String, String),
    Git(String),
}

fn block_cost(b: &Block) -> usize {
    match b {
        Block::ToolResult(l, o) => estimate_tokens(&format!("Tool result [{l}]:\n{o}")),
        Block::File(p, c) => estimate_tokens(&format!("File {p}:\n{c}")),
        Block::Git(g) => estimate_tokens(g),
    }
}

fn render_message(m: &Message) -> String {
    match m {
        Message::System(t) | Message::User(t) => t.clone(),
        Message::Assistant(a) => {
            let AssistantMessage { content, tool_calls } = a;
            format!("{}{:?}", content.clone().unwrap_or_default(), tool_calls)
        }
        Message::Tool(t) => t.content.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_orders_and_budgets() {
        let b = ContextBuilder::new(200);
        let input = ContextInput {
            instructions: "follow the rules".into(),
            relevant_files: vec![("a.rs".into(), "x".repeat(2000))],
            git_state: "git: branch main (clean)".into(),
            history: vec![Message::User("old question".into())],
            ..Default::default()
        };
        let ctx = b.build("fix it", &input);
        assert!(ctx.system_prompt.contains("Anvil"));
        assert!(ctx.system_prompt.contains("follow the rules"));
        assert!(ctx.estimated_tokens <= 400); // budget + request/system slack
        // Latest user request always present and last.
        assert_eq!(ctx.messages.last(), Some(&Message::User("fix it".into())));
        // Oversized file dropped under tiny budget, git kept.
        assert!(!ctx.messages.iter().any(|m| matches!(m, Message::User(t) if t.starts_with("File"))));
    }

    #[test]
    fn generous_budget_keeps_everything() {
        let b = ContextBuilder::new(32000);
        let input = ContextInput {
            relevant_files: vec![("a.rs".into(), "content".into())],
            tool_results: vec![("t1".into(), "out".into())],
            ..Default::default()
        };
        let ctx = b.build("hi", &input);
        assert!(ctx.messages.iter().any(|m| matches!(m, Message::Tool(_))));
        assert!(ctx.messages.iter().any(|m| matches!(m, Message::User(t) if t.contains("a.rs"))));
    }
}
