//! `App`: all TUI state + pure transitions (unit-testable, no terminal I/O).

use serde::{Deserialize, Serialize};

use anvil_core::{AgentEvent, ApprovalDecision, ApprovalRequest, ToolCall, ToolResult};

use crate::history;

/// One rendered block in the transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptEntry {
    User(String),
    Assistant(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    Error(String),
    System(String),
}

/// Result of interpreting an input line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlashAction {
    /// Forward to the agent as a user message.
    Send(String),
    /// Handled locally;QUIT the TUI.
    Quit,
    /// Handled locally; no agent traffic.
    Local(String),
    /// Async local work (git subprocess); result returns as `AgentEvent::Note`.
    GitDiff,
}

/// Single source of truth for slash commands: name + one-line description.
/// Used by completion, the help modal, and (via `interpret_input`) execution.
pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/help", "show this help"),
    ("/clear", "clear transcript"),
    ("/new", "start new session"),
    ("/connect", "set provider URL + model"),
    ("/history", "list saved sessions"),
    ("/model", "show current model"),
    ("/models", "list models"),
    ("/skills", "list skills"),
    ("/mcp", "list MCP servers"),
    ("/context", "show context usage"),
    ("/diff", "show current diff"),
    ("/status", "show session status"),
    ("/compact", "compact history"),
    ("/debug", "toggle under-the-hood event log"),
    ("/exit", "quit"),
    ("/quit", "quit (alias)"),
];

/// `/connect` form field indices.
pub const CONNECT_URL: usize = 0;
pub const CONNECT_MODEL: usize = 1;
pub const CONNECT_KEY: usize = 2;
pub const CONNECT_FIELDS: &[(&str, &str)] = &[
    ("URL", "OpenAI-compatible base URL"),
    ("Model", "model id"),
    ("API key", "saved locally, owner-only"),
];

/// Validated `/connect` settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectSettings {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

/// Interactive `/connect` form state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectForm {
    pub values: [String; 3],
    pub cursors: [usize; 3],
    pub focused: usize,
    pub error: Option<String>,
}

impl ConnectForm {
    fn new(base_url: &str, model: &str) -> Self {
        let mut form = Self {
            values: [base_url.to_string(), model.to_string(), String::new()],
            cursors: [0; 3],
            focused: 0,
            error: None,
        };
        form.cursors[0] = form.values[0].len();
        form.cursors[1] = form.values[1].len();
        form
    }
}

pub struct App {
    pub model: String,
    pub base_url: String,
    pub workspace: String,
    pub session_id: String,
    pub transcript: Vec<TranscriptEntry>,
    /// Current streaming assistant text (not yet committed).
    pub stream_buf: String,
    pub streaming: bool,
    pub thinking: bool,
    pub input: String,
    /// Byte index into `input` for the cursor.
    pub cursor: usize,
    /// Fire-and-forget background jobs (`/diff`, …) currently running.
    pub background_tasks: usize,
    /// Scroll offset in lines from the top of the wrapped transcript.
    pub scroll: u16,
    /// Whether the view is pinned to the bottom.
    pub follow: bool,
    pub pending_approval: Option<ApprovalRequest>,
    pub show_help: bool,
    /// Open `/connect` form, if any.
    pub connect_form: Option<ConnectForm>,
    /// Selected index in the `/` autocomplete palette.
    pub slash_selected: usize,
    /// Palette dismissed via Esc; re-armed on next edit.
    pub slash_dismissed: bool,
    pub status: String,
    pub should_quit: bool,
    /// Spinner frame for thinking indicator.
    pub spinner_tick: usize,
    /// Release version shown at the right side of the footer.
    /// Defaults to this crate's version; the binary overrides it with its own.
    pub version: String,
    /// When the current unit of work started (for the live elapsed timer).
    pub work_started: Option<std::time::Instant>,
    /// Under-the-hood event log overlay (`/debug`).
    pub debug: bool,
    pub event_log: std::collections::VecDeque<String>,
}

impl App {
    pub fn new(model: &str) -> Self {
        Self {
            model: model.to_string(),
            base_url: String::new(),
            workspace: String::new(),
            session_id: history::new_session_id(),
            transcript: vec![TranscriptEntry::System(
                "Welcome to Anvil. Type a message, `/` for commands, `/exit` to quit.".to_string(),
            )],
            stream_buf: String::new(),
            streaming: false,
            thinking: false,
            input: String::new(),
            cursor: 0,
            background_tasks: 0,
            scroll: 0,
            follow: true,
            pending_approval: None,
            show_help: false,
            connect_form: None,
            slash_selected: 0,
            slash_dismissed: false,
            status: String::new(),
            should_quit: false,
            spinner_tick: 0,
            version: env!("CARGO_PKG_VERSION").to_string(),
            work_started: None,
            debug: false,
            event_log: std::collections::VecDeque::new(),
        }
    }

    /// Seconds since the current work started, if any.
    pub fn elapsed_secs(&self) -> Option<u64> {
        self.work_started.map(|t| t.elapsed().as_secs())
    }

    /// Max entries kept in the debug event log.
    const EVENT_LOG_CAP: usize = 50;

    pub fn busy(&self) -> bool {
        self.thinking || self.streaming || self.background_tasks > 0
    }

    /// Transcript-level "working…" line: active work but no text streaming.
    pub fn show_working(&self) -> bool {
        (self.thinking || self.background_tasks > 0) && self.stream_buf.is_empty()
    }

    /// Mark a fire-and-forget background job started/finished (`/diff`).
    /// Completion arrives as `AgentEvent::Note`, which always ends one.
    pub fn begin_background(&mut self) {
        self.background_tasks = self.background_tasks.saturating_add(1);
        if self.work_started.is_none() {
            self.work_started = Some(std::time::Instant::now());
        }
        self.pin_to_bottom();
    }

    pub fn end_background(&mut self) {
        self.background_tasks = self.background_tasks.saturating_sub(1);
    }

    // ---- input editing ----

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.slash_selected = 0;
        self.slash_dismissed = false;
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.input[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.remove(prev);
        self.cursor = prev;
        self.slash_selected = 0;
        self.slash_dismissed = false;
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor > 0 {
            let prev = self.input[..self.cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.cursor = prev;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.cursor < self.input.len() {
            let next = self.input[self.cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.cursor + i)
                .unwrap_or(self.input.len());
            self.cursor = next;
        }
    }

    pub fn take_input(&mut self) -> String {
        let s = self.input.clone();
        self.input.clear();
        self.cursor = 0;
        self.slash_selected = 0;
        self.slash_dismissed = false;
        s
    }

    // ---- scrolling ----

    pub fn scroll_up(&mut self, n: u16) {
        self.follow = false;
        self.scroll = self.scroll.saturating_add(n);
    }

    pub fn scroll_down(&mut self, n: u16) {
        self.scroll = self.scroll.saturating_sub(n);
        if self.scroll == 0 {
            self.follow = true;
        }
    }

    pub fn pin_to_bottom(&mut self) {
        self.follow = true;
        self.scroll = 0;
    }

    pub fn tick_spinner(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
    }

    // ---- agent events (SPEC §26) ----

    fn log_event(&mut self, summary: String) {
        if self.event_log.len() >= Self::EVENT_LOG_CAP {
            self.event_log.pop_front();
        }
        self.event_log.push_back(summary);
    }

    fn describe_event(ev: &AgentEvent) -> String {
        match ev {
            AgentEvent::Thinking => "thinking (contacting model)".into(),
            AgentEvent::TextDelta(d) => format!("text +{} chars", d.chars().count()),
            AgentEvent::TextDone => "turn text complete".into(),
            AgentEvent::ToolStarted(c) => format!("tool start: {} {}", c.name, c.summary),
            AgentEvent::ToolOutput(r) => {
                format!("tool {} {}", if r.success { "ok" } else { "FAIL" }, r.summary)
            }
            AgentEvent::ApprovalRequired(r) => format!("approval needed: {}", r.preview),
            AgentEvent::Completed => "run complete".into(),
            AgentEvent::Cancelled => "run cancelled".into(),
            AgentEvent::Error(m) => format!("ERROR: {}", m.chars().take(300).collect::<String>()),
            AgentEvent::Usage { status } => format!("status: {status}"),
            AgentEvent::Note(m) => {
                format!("note: {}", m.chars().take(120).collect::<String>())
            }
        }
    }

    pub fn on_agent_event(&mut self, ev: AgentEvent) {
        self.log_event(Self::describe_event(&ev));
        match ev {
            AgentEvent::Thinking => {
                self.thinking = true;
                self.pin_to_bottom();
            }
            AgentEvent::TextDelta(d) => {
                self.thinking = false;
                self.streaming = true;
                self.stream_buf.push_str(&d);
                self.pin_to_bottom();
            }
            AgentEvent::TextDone => {
                self.flush_stream();
            }
            AgentEvent::ToolStarted(call) => {
                self.flush_stream();
                // Work continues (often the longest phase) — keep spinning.
                self.thinking = true;
                self.transcript.push(TranscriptEntry::ToolCall(call));
                self.pin_to_bottom();
            }
            AgentEvent::ToolOutput(res) => {
                self.transcript.push(TranscriptEntry::ToolResult(res));
                self.pin_to_bottom();
            }
            AgentEvent::ApprovalRequired(req) => {
                self.flush_stream();
                self.pending_approval = Some(req);
            }
            AgentEvent::Completed | AgentEvent::Cancelled => {
                self.flush_stream();
                self.thinking = false;
                self.streaming = false;
            }
            AgentEvent::Error(msg) => {
                self.flush_stream();
                self.thinking = false;
                self.streaming = false;
                self.transcript.push(TranscriptEntry::Error(msg));
                self.pin_to_bottom();
            }
            AgentEvent::Usage { status } => {
                self.status = status;
            }
            AgentEvent::Note(msg) => {
                self.end_background();
                self.transcript.push(TranscriptEntry::System(msg));
                self.pin_to_bottom();
            }
        }
        // Work over in every sense: stop the elapsed timer.
        if !self.busy() {
            self.work_started = None;
        }
    }

    fn flush_stream(&mut self) {
        if !self.stream_buf.is_empty() {
            let text = std::mem::take(&mut self.stream_buf);
            self.transcript.push(TranscriptEntry::Assistant(text));
        }
        self.streaming = false;
    }

    pub fn push_user(&mut self, text: &str) {
        self.flush_stream();
        self.transcript.push(TranscriptEntry::User(text.to_string()));
        self.thinking = true;
        self.work_started = Some(std::time::Instant::now());
        self.pin_to_bottom();
    }

    pub fn approval_decision(&mut self, d: ApprovalDecision) {
        self.pending_approval = None;
        let _ = d;
    }

    /// Emergency brake (Esc): drop all busy state immediately. The partial
    /// stream is kept in the transcript so no work output is lost.
    pub fn force_stop(&mut self) {
        self.flush_stream();
        self.thinking = false;
        self.streaming = false;
        self.background_tasks = 0;
        self.pending_approval = None;
        self.work_started = None;
        self.pin_to_bottom();
    }

    // ---- `/` autocomplete palette ----

    /// Palette is active while the user is typing a command token:
    /// input starts with `/` and there is no whitespace before the cursor.
    pub fn slash_active(&self) -> bool {
        if self.slash_dismissed || !self.input.starts_with('/') {
            return false;
        }
        let before = &self.input[..self.cursor.min(self.input.len())];
        !before.chars().any(|c| c.is_whitespace())
    }

    /// Commands matching the token typed so far (case-insensitive).
    pub fn slash_matches(&self) -> Vec<(&'static str, &'static str)> {
        if !self.slash_active() {
            return Vec::new();
        }
        let prefix = self.input[..self.cursor.min(self.input.len())].to_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|(name, _)| name.starts_with(prefix.as_str()))
            .copied()
            .collect()
    }

    fn slash_count(&self) -> usize {
        self.slash_matches().len()
    }

    pub fn slash_next(&mut self) {
        let n = self.slash_count();
        if n > 0 {
            self.slash_selected = (self.slash_selected + 1) % n;
        }
    }

    pub fn slash_prev(&mut self) {
        let n = self.slash_count();
        if n > 0 {
            self.slash_selected = (self.slash_selected + n - 1) % n;
        }
    }

    pub fn slash_dismiss(&mut self) {
        self.slash_dismissed = true;
    }

    /// Complete the token before the cursor with the selected command.
    /// Returns false when there is nothing to complete.
    pub fn slash_accept(&mut self) -> bool {
        let matches = self.slash_matches();
        if matches.is_empty() {
            return false;
        }
        let cmd = matches[self.slash_selected.min(matches.len() - 1)].0;
        let end = self.cursor.min(self.input.len());
        self.input.replace_range(..end, cmd);
        self.cursor = cmd.len();
        self.slash_selected = 0;
        true
    }

    /// `Enter` with the palette open: run the highlighted command.
    /// If the typed token is already an exact command it is left alone;
    /// otherwise it is completed to the current selection first.
    pub fn slash_confirm(&mut self) {
        let token = self.input.trim().split_whitespace().next().unwrap_or("");
        let exact = SLASH_COMMANDS.iter().any(|(name, _)| *name == token);
        if !exact {
            let _ = self.slash_accept();
        }
    }

    // ---- `/connect` form ----

    pub fn open_connect(&mut self) {
        self.connect_form = Some(ConnectForm::new(&self.base_url, &self.model));
    }

    fn form_mut(&mut self) -> Option<&mut ConnectForm> {
        self.connect_form.as_mut()
    }

    pub fn connect_insert(&mut self, c: char) {
        if let Some(f) = self.form_mut() {
            let i = f.focused;
            f.values[i].insert(f.cursors[i], c);
            f.cursors[i] += c.len_utf8();
            f.error = None;
        }
    }

    pub fn connect_backspace(&mut self) {
        if let Some(f) = self.form_mut() {
            let i = f.focused;
            if f.cursors[i] == 0 {
                return;
            }
            let prev = f.values[i][..f.cursors[i]]
                .char_indices()
                .last()
                .map(|(ix, _)| ix)
                .unwrap_or(0);
            f.values[i].remove(prev);
            f.cursors[i] = prev;
            f.error = None;
        }
    }

    pub fn connect_left(&mut self) {
        if let Some(f) = self.form_mut() {
            let i = f.focused;
            if f.cursors[i] > 0 {
                f.cursors[i] = f.values[i][..f.cursors[i]]
                    .char_indices()
                    .last()
                    .map(|(ix, _)| ix)
                    .unwrap_or(0);
            }
        }
    }

    pub fn connect_right(&mut self) {
        if let Some(f) = self.form_mut() {
            let i = f.focused;
            if f.cursors[i] < f.values[i].len() {
                f.cursors[i] = f.values[i][f.cursors[i]..]
                    .char_indices()
                    .nth(1)
                    .map(|(ix, _)| f.cursors[i] + ix)
                    .unwrap_or(f.values[i].len());
            }
        }
    }

    pub fn connect_next(&mut self) {
        if let Some(f) = self.form_mut() {
            f.focused = (f.focused + 1) % CONNECT_FIELDS.len();
        }
    }

    pub fn connect_prev(&mut self) {
        if let Some(f) = self.form_mut() {
            f.focused = (f.focused + CONNECT_FIELDS.len() - 1) % CONNECT_FIELDS.len();
        }
    }

    /// Validate the form. On success the caller applies + persists the
    /// settings; on failure the error is shown in the form.
    pub fn connect_submit(&mut self) -> Result<ConnectSettings, String> {
        let form = self.form_mut().ok_or_else(|| "connect form is not open".to_string())?;
        let url = form.values[CONNECT_URL].trim().to_string();
        let model = form.values[CONNECT_MODEL].trim().to_string();
        let api_key = form.values[CONNECT_KEY].trim().to_string();
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            let e = "Base URL must start with http:// or https://".to_string();
            form.error = Some(e.clone());
            return Err(e);
        }
        if model.is_empty() {
            let e = "Model must not be empty".to_string();
            form.error = Some(e.clone());
            return Err(e);
        }
        Ok(ConnectSettings {
            base_url: url.trim_end_matches('/').to_string(),
            model,
            api_key,
        })
    }

    pub fn apply_connect(&mut self, s: &ConnectSettings) {
        self.base_url = s.base_url.clone();
        self.model = s.model.clone();
        self.status = format!("model: {}  tokens: —  latency: —", s.model);
        self.connect_form = None;
        self.pin_to_bottom();
    }

    /// Masked rendering of the API key (same length, no secrets on screen).
    pub fn mask_key(key: &str) -> String {
        "•".repeat(key.chars().count())
    }

    // ---- session history (SPEC §19) ----

    pub fn snapshot(&self) -> history::StoredSession {
        history::StoredSession {
            version: 1,
            id: self.session_id.clone(),
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            updated_at: history::now_secs(),
            messages: self.transcript.clone(),
        }
    }

    pub fn restore(&mut self, s: history::StoredSession) {
        self.session_id = s.id;
        self.model = s.model;
        self.base_url = s.base_url;
        self.transcript = s.messages;
        self.stream_buf.clear();
        self.streaming = false;
        self.thinking = false;
        self.status = format!("model: {}  tokens: —  latency: —", self.model);
        self.pin_to_bottom();
    }

    fn history_listing() -> String {
        let sessions = history::list();
        if sessions.is_empty() {
            return "No saved sessions yet.".to_string();
        }
        let mut out = vec![format!("{} saved session(s):", sessions.len())];
        for m in sessions.iter().take(10) {
            out.push(format!("  {}  {}  {} msg", m.id, m.model, m.messages));
        }
        out.join("\n")
    }

    fn context_summary(&self) -> String {
        let chars: usize = self.transcript.iter().map(|e| match e {
            TranscriptEntry::User(t)
            | TranscriptEntry::Assistant(t)
            | TranscriptEntry::Error(t)
            | TranscriptEntry::System(t) => t.len(),
            TranscriptEntry::ToolCall(c) => c.summary.len(),
            TranscriptEntry::ToolResult(r) => r.summary.len() + r.output.len(),
        }).sum();
        // ~4 chars per token heuristic for the estimate.
        format!(
            "session {}  messages: {}  ~{} chars (~{} tokens est.)",
            self.session_id,
            self.transcript.len(),
            chars,
            chars / 4
        )
    }

    fn models_listing(&self) -> String {
        let current = if self.model.is_empty() { "(unset)" } else { &self.model };
        let via = if self.base_url.is_empty() {
            "(unset — use /connect)".to_string()
        } else {
            format!("{} (+ /chat/completions)", self.base_url)
        };
        let mut out = vec![format!("* {current}  (current, via {via})")];
        if current != anvil_core::DEFAULT_MODEL {
            out.push(format!(
                "  {}  (built-in default, OpenCode Zen free)",
                anvil_core::DEFAULT_MODEL
            ));
        }
        out.push("tip: /connect to point Anvil at any OpenAI-compatible provider.".to_string());
        out.join("\n")
    }

    fn skills_listing() -> String {
        let workspace = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let skills = anvil_skills::all(&workspace);
        if skills.is_empty() {
            "No skills found. Add .anvil/skills/<name>/SKILL.md (project) or ~/.config/anvil/skills/<name>/SKILL.md (global).".to_string()
        } else {
            let mut out = vec![format!("skills ({}):", skills.len())];
            for s in &skills {
                let scope = match s.source {
                    anvil_skills::SkillSource::Builtin => "builtin",
                    anvil_skills::SkillSource::Project => "project",
                    anvil_skills::SkillSource::Global => "global",
                };
                let desc = if s.description.is_empty() { "(no description)" } else { &s.description };
                out.push(format!("  {:<20} [{scope}] {desc}", s.name));
            }
            out.join("\n")
        }
    }

    fn mcp_config_path() -> std::path::PathBuf {
        if let Ok(p) = std::env::var("ANVIL_CONFIG") {
            if !p.trim().is_empty() {
                return std::path::PathBuf::from(p);
            }
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home).join(".config/anvil/config.yaml")
    }

    fn mcp_listing() -> String {
        let path = Self::mcp_config_path();
        let has_section = std::fs::read_to_string(&path)
            .map(|c| {
                c.lines().any(|l| {
                    !(l.starts_with(' ') || l.starts_with('\t'))
                        && l.trim().starts_with("mcp:")
                })
            })
            .unwrap_or(false);
        if has_section {
            format!(
                "MCP servers declared in {} (live tool execution lands in Phase 7).",
                path.display()
            )
        } else {
            format!(
                "No MCP servers configured. Add an `mcp:` section to {} (client lands in Phase 7).",
                path.display()
            )
        }
    }

    // ---- slash commands (SPEC §20) ----
    // Handled by Anvil itself; never sent to the model.

    pub fn interpret_input(&mut self, raw: String) -> SlashAction {
        let line = raw.trim().to_string();
        if line.is_empty() {
            return SlashAction::Local(String::new());
        }
        if !line.starts_with('/') {
            return SlashAction::Send(line);
        }
        let cmd = line.split_whitespace().next().unwrap_or("");
        match cmd {
            "/exit" | "/quit" => {
                self.should_quit = true;
                SlashAction::Quit
            }
            "/clear" => {
                self.transcript.clear();
                self.stream_buf.clear();
                self.streaming = false;
                self.pin_to_bottom();
                SlashAction::Local("Transcript cleared.".to_string())
            }
            "/new" => {
                self.transcript.clear();
                self.stream_buf.clear();
                self.streaming = false;
                self.session_id = history::new_session_id();
                self.transcript.push(TranscriptEntry::System(
                    "New session started.".to_string(),
                ));
                self.pin_to_bottom();
                SlashAction::Local("New session started.".to_string())
            }
            "/connect" => {
                self.open_connect();
                SlashAction::Local(String::new())
            }
            "/debug" => {
                self.debug = !self.debug;
                SlashAction::Local(format!(
                    "under-the-hood event log {}.",
                    if self.debug { "shown" } else { "hidden" }
                ))
            }
            "/history" => SlashAction::Local(Self::history_listing()),
            "/status" => SlashAction::Local(format!(
                "session: {}\nmodel: {} via {}\nbusy: {}",
                self.session_id,
                if self.model.is_empty() { "(unset)" } else { &self.model },
                if self.base_url.is_empty() { "(unset — use /connect)" } else { &self.base_url },
                self.busy(),
            )),
            "/context" => SlashAction::Local(Self::context_summary(&self)),
            "/model" => SlashAction::Local(format!(
                "model: {} via {} (+ /chat/completions). Use /connect to change.",
                if self.model.is_empty() { "(unset)" } else { &self.model },
                if self.base_url.is_empty() { "(unset)" } else { &self.base_url },
            )),
            "/models" => SlashAction::Local(Self::models_listing(&self)),
            "/skills" => SlashAction::Local(Self::skills_listing()),
            "/mcp" => SlashAction::Local(Self::mcp_listing()),
            "/diff" => SlashAction::GitDiff,
            "/compact" => {
                const KEEP: usize = 20;
                let n = self.transcript.len();
                if n > KEEP + 1 {
                    let dropped = n - KEEP;
                    self.transcript = std::mem::take(&mut self.transcript)
                        .into_iter()
                        .skip(dropped)
                        .collect();
                    self.pin_to_bottom();
                    SlashAction::Local(format!(
                        "Compacted transcript: dropped {dropped} older entries, kept last {KEEP}."
                    ))
                } else {
                    SlashAction::Local("Nothing to compact: transcript is already short.".to_string())
                }
            }
            "/help" => {
                self.show_help = true;
                SlashAction::Local("help".to_string())
            }
            _ => SlashAction::Local(format!("Unknown command: {cmd}. Try /help.")),
        }
    }

    pub fn help_text() -> &'static str {
        "/help /clear /new /connect /history /model /models /skills /mcp /context /diff /status /compact /debug /exit"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn streaming_deltas_accumulate_and_flush_on_tool() {
        let mut app = App::new("cpp-agent");
        app.on_agent_event(AgentEvent::Thinking);
        assert!(app.busy());
        app.on_agent_event(AgentEvent::TextDelta("I'll inspect ".to_string()));
        app.on_agent_event(AgentEvent::TextDelta("the parser.".to_string()));
        assert_eq!(app.stream_buf, "I'll inspect the parser.");
        let call = ToolCall::new("t1", "anvil.read_file", json!({"path": "src/a.cpp"}), "read_file src/a.cpp");
        app.on_agent_event(AgentEvent::ToolStarted(call.clone()));
        // stream flushed into transcript before the tool entry
        assert!(app.stream_buf.is_empty());
        assert!(matches!(app.transcript.last(), Some(TranscriptEntry::ToolCall(_))));
        assert!(app.transcript.iter().any(|e| matches!(e, TranscriptEntry::Assistant(_))));
        // spinner stays on through tool execution (not just user chat)
        assert!(app.busy());
        assert!(app.show_working());
        app.on_agent_event(AgentEvent::Completed);
        assert!(!app.busy());
        assert!(!app.show_working());
    }

    #[test]
    fn force_stop_clears_all_busy_state_but_keeps_partial_text() {
        let mut app = App::new("m");
        app.on_agent_event(AgentEvent::TextDelta("partial".into()));
        app.begin_background();
        app.force_stop();
        assert!(!app.busy());
        assert!(!app.show_working());
        assert!(app.pending_approval.is_none());
        // partial stream preserved as an assistant message
        assert!(app.transcript.iter().any(|e| matches!(
            e, TranscriptEntry::Assistant(t) if t == "partial"
        )));
    }

    #[test]
    fn background_tasks_drive_busy_until_note() {
        let mut app = App::new("m");
        assert!(!app.busy());
        app.begin_background();
        assert!(app.busy());
        assert!(app.show_working());
        // A Note from elsewhere (e.g. skill load) never drives it negative…
        app.on_agent_event(AgentEvent::Note("x".into()));
        assert!(!app.busy());
        app.on_agent_event(AgentEvent::Note("y".into()));
        assert!(!app.busy());
        // …and Notes don't clear real run state.
        app.on_agent_event(AgentEvent::Thinking);
        app.begin_background();
        app.on_agent_event(AgentEvent::Note("z".into()));
        assert!(app.busy());
        assert!(app.show_working());
    }

    #[test]
    fn debug_toggle_logs_events_and_caps() {
        let mut app = App::new("m");
        assert!(!app.debug);
        let action = app.interpret_input("/debug".into());
        assert!(app.debug);
        assert!(matches!(action, SlashAction::Local(_)));
        app.on_agent_event(AgentEvent::Thinking);
        app.on_agent_event(AgentEvent::TextDelta("hi".into()));
        app.on_agent_event(AgentEvent::Error("boom".into()));
        assert!(app.event_log.iter().any(|e| e.contains("thinking")));
        assert!(app.event_log.iter().any(|e| e.contains("ERROR: boom")));
        for _ in 0..60 {
            app.on_agent_event(AgentEvent::Thinking);
        }
        assert_eq!(app.event_log.len(), 50);
    }

    #[test]
    fn elapsed_timer_runs_while_busy() {
        let mut app = App::new("m");
        assert_eq!(app.elapsed_secs(), None);
        app.push_user("hi");
        assert!(app.elapsed_secs().is_some());
        app.on_agent_event(AgentEvent::Completed);
        assert_eq!(app.elapsed_secs(), None);
    }

    #[test]
    fn slash_commands_never_forwarded_as_send() {
        let mut app = App::new("m");
        assert!(matches!(app.interpret_input("/clear".into()), SlashAction::Local(_)));
        assert!(matches!(app.interpret_input("/nope".into()), SlashAction::Local(_)));
        assert!(matches!(app.interpret_input("hello".into()), SlashAction::Send(_)));
        let mut app2 = App::new("m");
        assert!(matches!(app2.interpret_input("/exit".into()), SlashAction::Quit));
        assert!(app2.should_quit);
    }

    #[test]
    fn approval_blocks_until_decision() {
        let mut app = App::new("m");
        let req = ApprovalRequest {
            tool_call: ToolCall::new("t1", "anvil.shell", serde_json::json!({"command": "cargo test"}), "run: cargo test"),
            preview: "cargo test".to_string(),
        };
        app.on_agent_event(AgentEvent::ApprovalRequired(req));
        assert!(app.pending_approval.is_some());
        app.approval_decision(ApprovalDecision::AllowOnce);
        assert!(app.pending_approval.is_none());
    }

    #[test]
    fn error_resets_busy_and_records() {
        let mut app = App::new("m");
        app.on_agent_event(AgentEvent::TextDelta("partial".into()));
        app.on_agent_event(AgentEvent::Error("boom".into()));
        assert!(!app.busy());
        assert!(matches!(app.transcript.last(), Some(TranscriptEntry::Error(_))));
    }

    #[test]
    fn input_editing_multiline() {
        let mut app = App::new("m");
        for c in "hi".chars() {
            app.insert_char(c);
        }
        app.insert_newline();
        app.insert_char('!');
        assert_eq!(app.input, "hi\n!");
        app.move_cursor_left();
        app.backspace();
        assert_eq!(app.input, "hi!");
    }

    #[test]
    fn slash_palette_filters_and_completes() {
        let mut app = App::new("m");
        assert!(app.slash_matches().is_empty());
        for c in "/mo".chars() {
            app.insert_char(c);
        }
        let names: Vec<_> = app.slash_matches().iter().map(|(n, _)| *n).collect();
        assert_eq!(names, vec!["/model", "/models"]);
        app.slash_next();
        app.slash_next(); // wraps
        assert!(app.slash_accept());
        assert_eq!(app.input, "/model");
    }

    #[test]
    fn slash_palette_hides_after_space_or_dismiss() {
        let mut app = App::new("m");
        for c in "/help x".chars() {
            app.insert_char(c);
        }
        assert!(app.slash_matches().is_empty());
        let mut app2 = App::new("m");
        for c in "/he".chars() {
            app2.insert_char(c);
        }
        assert_eq!(app2.slash_matches().len(), 1);
        app2.slash_dismiss();
        assert!(app2.slash_matches().is_empty());
        app2.insert_char('l'); // next edit re-arms
        assert_eq!(app2.slash_matches().len(), 1);
    }

    #[test]
    fn connect_validates_and_applies() {
        let mut app = App::new("m");
        app.base_url = "https://old.example/v1".to_string();
        assert!(matches!(app.interpret_input("/connect".into()), SlashAction::Local(_)));
        assert!(app.connect_form.is_some());
        // prefilled from current settings
        assert!(app.connect_submit().is_ok());
        // bad URL rejected, form stays open with error
        app.connect_form.as_mut().unwrap().values[CONNECT_URL] = "ftp://x".to_string();
        assert!(app.connect_submit().is_err());
        assert!(app.connect_form.is_some());
        assert!(app.connect_form.as_ref().unwrap().error.is_some());
        // fix + trailing slash trimmed
        app.connect_form.as_mut().unwrap().values[CONNECT_URL] = "https://new.example/v1/".to_string();
        let s = app.connect_submit().unwrap();
        assert_eq!(s.base_url, "https://new.example/v1");
        app.apply_connect(&s);
        assert_eq!(app.model, "m");
        assert_eq!(app.base_url, "https://new.example/v1");
        assert!(app.connect_form.is_none());
    }

    #[test]
    fn connect_rejects_empty_model_and_cycles_fields() {
        let mut app = App::new("m");
        app.open_connect();
        app.connect_form.as_mut().unwrap().values[CONNECT_MODEL] = "   ".to_string();
        assert!(app.connect_submit().is_err());
        app.connect_next();
        app.connect_next();
        assert_eq!(app.connect_form.as_ref().unwrap().focused, CONNECT_KEY);
        app.connect_prev();
        assert_eq!(app.connect_form.as_ref().unwrap().focused, CONNECT_MODEL);
        assert_eq!(App::mask_key("secret"), "••••••");
    }

    #[test]
    fn session_snapshot_roundtrip() {
        let mut app = App::new("m");
        app.base_url = "https://x/v1".to_string();
        app.push_user("hello");
        let snap = app.snapshot();
        assert_eq!(snap.model, "m");
        assert_eq!(snap.messages.len(), 2); // welcome + user
        let mut app2 = App::new("other");
        app2.restore(snap);
        assert_eq!(app2.model, "m");
        assert_eq!(app2.transcript.len(), 2);
    }
}
