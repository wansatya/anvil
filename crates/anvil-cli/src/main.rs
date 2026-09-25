//! `anvil` binary (SPEC §4). Default invocation starts the interactive TUI.

mod config;

use std::io::Stdout;
use std::sync::Arc;
use std::time::Duration;

use anvil_core::agent::{Agent, AgentConfig, ApprovalGate, ApprovalJob};
use anvil_core::{AgentEvent, ApprovalDecision, Workspace};
use anvil_tui::{App, SlashAction};
use anvil_tui::app::ConnectSettings;
use clap::{Parser, Subcommand};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::{mpsc, oneshot, watch};


#[derive(Parser)]
#[command(name = "anvil", about = "Anvil — terminal-native engineering agent")]
struct Cli {
    /// Resume a saved session by id (see `/history`).
    #[arg(long)]
    session: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize Anvil configuration in the current project.
    Init,
    /// Execute a task without entering the interactive TUI.
    Run {
        prompt: String,
        /// Auto-allow tools that would otherwise ask for approval.
        #[arg(long)]
        yes: bool,
    },
    /// List configured models.
    Models,
    /// List available skills.
    Skills,
    /// List configured MCP servers.
    Mcp,
    /// Show effective configuration.
    Config,
    /// Check configuration, provider connectivity and tool availability.
    Doctor,
    /// Show version.
    Version,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        None => run_tui(cli.session).await,
        Some(Command::Init) => cmd_init(),
        Some(Command::Run { prompt, yes }) => cmd_run(prompt, yes).await,
        Some(Command::Models) => {
            let cfg = config::load();
            println!("current:   {} (via {})", cfg.model, cfg.base_url);
            if cfg.model != config::DEFAULT_MODEL {
                println!("default:   {} (OpenCode Zen free)", config::DEFAULT_MODEL);
            }
            println!("tip: /connect (or ANVIL_MODEL) to change.");
            Ok(())
        }
        Some(Command::Skills) => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let skills = anvil_skills::all(&cwd);
            if skills.is_empty() {
                println!("No skills found.");
            } else {
                for s in &skills {
                    let scope = match s.source {
                        anvil_skills::SkillSource::Builtin => "builtin",
                        anvil_skills::SkillSource::Project => "project",
                        anvil_skills::SkillSource::Global => "global",
                    };
                    let desc = if s.description.is_empty() { "(no description)" } else { &s.description };
                    println!("{:24} [{scope}] {desc}", s.name);
                }
            }
            Ok(())
        }
        Some(Command::Mcp) => {
            let servers = config::mcp_servers();
            if servers.is_empty() {
                println!("No MCP servers configured. Add an `mcp:` section to ~/.config/anvil/config.yaml.");
            } else {
                for (name, s) in &servers {
                    println!("{name}: {} {}", s.command, s.args.join(" "));
                }
            }
            Ok(())
        }
        Some(Command::Config) => {
            let cfg = config::load();
            println!("{}", serde_json::to_string_pretty(&cfg)?);
            Ok(())
        }
        Some(Command::Doctor) => {
            let cfg = config::load();
            println!("model: {}", cfg.model);
            println!("base_url: {} (+ /chat/completions)", cfg.base_url);
            println!("provider: openai-compatible (OpenCode Zen)");
            println!("api_key: {}", if cfg.has_api_key { "set (redacted)" } else { "missing (set ANVIL_API_KEY or OPENCODE_ZEN_API_KEY)" });
            println!("workspace: {}", cfg.workspace);
            println!("tools: {} native + {} mcp servers declared",
                anvil_tools::registry::native_registry().specs().len(),
                config::mcp_servers().len());
            Ok(())
        }
        Some(Command::Version) => {
            println!("anvil {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime: real agent loop wired to provider + tools (SPEC §3, §6).
// ---------------------------------------------------------------------------

/// Hot-swappable provider so `/connect` applies without restarting the TUI.
#[derive(Clone)]
struct HotSwapProvider {
    inner: Arc<std::sync::RwLock<Arc<dyn anvil_model::ModelProvider>>>,
}

impl HotSwapProvider {
    fn new(first: Arc<dyn anvil_model::ModelProvider>) -> Self {
        Self { inner: Arc::new(std::sync::RwLock::new(first)) }
    }

    fn swap(&self, next: Arc<dyn anvil_model::ModelProvider>) {
        *self.inner.write().unwrap() = next;
    }

    /// Fresh OpenAI-compatible provider from current config + env.
    /// Reads env at call time so `/connect` (which sets `ANVIL_API_KEY`
    /// in-process) takes effect immediately.
    fn build_connection() -> Arc<dyn anvil_model::ModelProvider> {
        let cfg = config::load();
        Arc::new(anvil_model::OpenAiCompatible::new(&cfg.base_url, &config::api_key()))
    }
}

#[async_trait::async_trait]
impl anvil_model::ModelProvider for HotSwapProvider {
    fn name(&self) -> &str {
        "hot-swap"
    }

    async fn chat(
        &self,
        request: anvil_model::ChatRequest,
        cancel: watch::Receiver<bool>,
    ) -> Result<anvil_model::ChatStream, anvil_model::ModelError> {
        // Clone out from under the lock; the guard must not cross `.await`.
        let inner = { self.inner.read().unwrap().clone() };
        inner.chat(request, cancel).await
    }
}

struct BuiltAgent {
    agent: Arc<Agent>,
    swapper: HotSwapProvider,
    warnings: Vec<String>,
}

/// Build the full runtime: provider, native + MCP tools, context, skills.
async fn build_agent() -> anyhow::Result<BuiltAgent> {
    let cfg = config::load();
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let workspace = Workspace::discover(&cwd);

    let mut registry = anvil_tools::registry::native_registry();
    let mut warnings = Vec::new();
    for (name, decl) in config::mcp_servers() {
        let spawn = tokio::time::timeout(
            Duration::from_secs(20),
            anvil_mcp::McpClient::spawn(&anvil_mcp::McpConfig::new(&decl.command, decl.args.clone())),
        )
        .await;
        match spawn {
            Ok(Ok(client)) => {
                let client = Arc::new(client);
                match client.list_tools().await {
                    Ok(tools) => {
                        for def in tools {
                            registry.register(anvil_mcp::McpToolAdapter::new(&name, def, client.clone()));
                        }
                    }
                    Err(e) => warnings.push(format!("MCP `{name}`: tools/list failed ({e})")),
                }
            }
            Ok(Err(e)) => warnings.push(format!("MCP `{name}`: failed to start ({e})")),
            Err(_) => warnings.push(format!("MCP `{name}`: timed out starting")),
        }
    }

    let swapper = HotSwapProvider::new(HotSwapProvider::build_connection());
    let agent = Agent {
        config: AgentConfig {
            model: cfg.model.clone(),
            max_iterations: cfg.max_iterations,
            context_max_tokens: 32000,
            temperature: 0.2,
        },
        provider: Arc::new(swapper.clone()) as Arc<dyn anvil_model::ModelProvider>,
        tools: Arc::new(registry),
        tool_ctx: anvil_tools::registry::ToolContext::new(&workspace.root),
        permissions: std::sync::Mutex::new(anvil_core::PermissionSet::defaults()),
        skills: anvil_skills::all(&workspace.root),
        workspace,
    };
    Ok(BuiltAgent { agent: Arc::new(agent), swapper, warnings })
}

/// Per-session UI-side runtime state.
struct Runtime {
    agent: Arc<Agent>,
    swapper: HotSwapProvider,
    history: Arc<tokio::sync::Mutex<Vec<anvil_model::Message>>>,
    approvals_tx: mpsc::UnboundedSender<ApprovalJob>,
    cancel_tx: Option<watch::Sender<bool>>,
    approval_reply: Option<oneshot::Sender<ApprovalDecision>>,
    task: Option<tokio::task::JoinHandle<()>>,
    running: bool,
}

fn spawn_agent_task(rt: &mut Runtime, agent_tx: &mpsc::UnboundedSender<AgentEvent>, prompt: String) {
    let agent = rt.agent.clone();
    let history = rt.history.clone();
    let approvals = ApprovalGate::new(rt.approvals_tx.clone());
    let (cancel_tx, cancel_rx) = watch::channel(false);
    rt.cancel_tx = Some(cancel_tx);
    rt.running = true;
    let tx = agent_tx.clone();
    rt.task = Some(tokio::spawn(async move {
        // Completion state arrives as events; nothing to send on success.
        // A panic must ALSO surface as an event — otherwise the UI sticks
        // on "working…" with input blocked (same as a silent error).
        let run = std::panic::AssertUnwindSafe(async {
            let mut h = history.lock().await;
            agent.run(prompt, &mut h, tx.clone(), cancel_rx, approvals).await
        });
        if futures_util::FutureExt::catch_unwind(run).await.is_err() {
            let _ = tx.send(AgentEvent::Error(
                "agent task panicked — run aborted; please report this bug.".into(),
            ));
        }
    }));
}

/// Emergency brake (Esc while work runs): ask cooperatively AND abort the
/// task, so a stuck provider call or tool can never outlive the keypress.
/// Child processes die with the task (kill_on_drop); the partial stream
/// is kept in the transcript.
fn force_stop(app: &mut App, rt: &mut Runtime) {
    if let Some(tx) = rt.cancel_tx.take() {
        let _ = tx.send(true);
    }
    if let Some(handle) = rt.task.take() {
        handle.abort();
    }
    drop(rt.approval_reply.take());
    app.force_stop();
    app.transcript.push(anvil_tui::app::TranscriptEntry::System(
        "Stopped.".to_string(),
    ));
    app.pin_to_bottom();
    rt.running = false;
    persist(app);
}

// ---------------------------------------------------------------------------
// TUI
// ---------------------------------------------------------------------------

async fn run_tui(resume: Option<String>) -> anyhow::Result<()> {
    let cfg = config::load();
    let mut app = App::new(&cfg.model);
    app.version = env!("CARGO_PKG_VERSION").to_string();
    app.base_url = cfg.base_url.clone();
    app.workspace = cfg.workspace.clone();
    app.status = format!("model: {}  tokens: —  latency: —", cfg.model);

    let BuiltAgent { agent, swapper, warnings } = {
        // MCP servers can take seconds to spawn; the TUI isn't up yet,
        // so say so on plain stderr instead of hanging silently.
        if config::mcp_servers().is_empty() {
            build_agent().await?
        } else {
            eprintln!("anvil: starting MCP servers…");
            build_agent().await?
        }
    };
    let (ap_tx, mut ap_rx) = mpsc::unbounded_channel::<ApprovalJob>();
    let mut rt = Runtime {
        agent,
        swapper,
        history: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        approvals_tx: ap_tx,
        cancel_tx: None,
        approval_reply: None,
        task: None,
        running: false,
    };

    // Session resume (SPEC §19): transcript + model history.
    if let Some(id) = resume {
        match anvil_tui::history::load(&id) {
            Ok(stored) => {
                let model_len = anvil_tui::history::to_model_messages(&stored).len();
                // Fill history synchronously: nothing else can hold the lock yet.
                *rt.history.try_lock().unwrap() = anvil_tui::history::to_model_messages(&stored);
                app.restore(stored);
                app.transcript.push(anvil_tui::app::TranscriptEntry::System(format!(
                    "Resumed session {id} ({model_len} prior messages)."
                )));
            }
            Err(e) => app.transcript.push(anvil_tui::app::TranscriptEntry::System(format!(
                "Could not resume session {id}: {e}"
            ))),
        }
    }
    for w in warnings {
        app.transcript.push(anvil_tui::app::TranscriptEntry::System(format!("Warning: {w}")));
    }
    if config::api_key().is_empty() {
        app.transcript.push(anvil_tui::app::TranscriptEntry::System(
            "No API key configured — set ANVIL_API_KEY (or OPENCODE_ZEN_API_KEY) or use /connect.".to_string(),
        ));
    }
    app.pin_to_bottom();

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    // Agent -> TUI channel: the real loop emits AgentEvents (SPEC §26).
    let (agent_tx, mut agent_rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

    let res = run_loop(&mut terminal, &mut app, &mut rt, &mut agent_rx, &mut ap_rx, &agent_tx).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
    rt: &mut Runtime,
    agent_rx: &mut tokio::sync::mpsc::UnboundedReceiver<AgentEvent>,
    ap_rx: &mut tokio::sync::mpsc::UnboundedReceiver<ApprovalJob>,
    agent_tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> anyhow::Result<()> {
    agent_tx.send(AgentEvent::Usage {
        status: format!("model: {}  tokens: —  latency: —", app.model),
    })?;

    // Keys arrive via an async event stream so input is handled the moment
    // it is typed (no polling delay); the ticker only drives repaints.
    let mut keys = EventStream::new();
    let mut ticker = tokio::time::interval(Duration::from_millis(100));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        app.tick_spinner();
        terminal.draw(|f| anvil_tui::ui::render(f, app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            biased;
            Some(ev) = agent_rx.recv() => {
                if matches!(ev, AgentEvent::Completed | AgentEvent::Error(_) | AgentEvent::Cancelled) {
                    rt.running = false;
                    rt.cancel_tx = None;
                    // Finished handle; drop it so a later Esc can't touch it.
                    rt.task.take();
                }
                app.on_agent_event(ev);
                persist(app);
            }
            Some(job) = ap_rx.recv() => {
                rt.approval_reply = Some(job.reply);
                app.on_agent_event(AgentEvent::ApprovalRequired(job.request));
                persist(app);
            }
            key_ev = keys.next() => {
                if let Some(Ok(Event::Key(key))) = key_ev {
                    if handle_key(app, rt, key.code, key.modifiers, agent_tx) {
                        persist(app);
                        break;
                    }
                }
                // Resize and other events: fall through to repaint.
            }
            _ = ticker.tick() => {}
        }
    }
    // Quitting with work in flight (Ctrl+C mid-run): abort so no agent
    // task or child process outlives the TUI.
    if let Some(handle) = rt.task.take() {
        handle.abort();
    }
    drop(rt.approval_reply.take());
    Ok(())
}

/// Returns true when the TUI should exit.
fn handle_key(
    app: &mut App,
    rt: &mut Runtime,
    code: KeyCode,
    mods: KeyModifiers,
    agent_tx: &tokio::sync::mpsc::UnboundedSender<AgentEvent>,
) -> bool {
    // Ctrl+C always exits the TUI, even mid-run (the run is aborted on
    // the way out). Esc is the emergency brake for work; see below.
    if code == KeyCode::Char('c') && mods.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return true;
    }

    // --- approval modal has focus (SPEC §12) ---
    // The decision resolves the agent's paused approval gate directly.
    // Esc here means Deny (as the modal hint says).
    if app.pending_approval.is_some() {
        let decision = match code {
            KeyCode::Enter => Some(ApprovalDecision::AllowOnce),
            KeyCode::Esc => Some(ApprovalDecision::Deny),
            KeyCode::Char('a') | KeyCode::Char('A') => Some(ApprovalDecision::AllowAlways),
            _ => None,
        };
        if let Some(d) = decision {
            app.approval_decision(d);
            if let Some(reply) = rt.approval_reply.take() {
                let _ = reply.send(d);
            }
        }
        return false;
    }

    // --- help modal ---
    if app.show_help {
        match code {
            KeyCode::Esc | KeyCode::Char('?') => app.show_help = false,
            _ => app.show_help = false,
        }
        return false;
    }

    // --- `/connect` form has focus ---
    if app.connect_form.is_some() {
        match code {
            KeyCode::Esc => app.connect_form = None,
            KeyCode::Tab | KeyCode::BackTab => {
                if code == KeyCode::BackTab {
                    app.connect_prev();
                } else if mods.contains(KeyModifiers::SHIFT) {
                    app.connect_prev();
                } else {
                    app.connect_next();
                }
            }
            KeyCode::Up => app.connect_prev(),
            KeyCode::Down => app.connect_next(),
            KeyCode::Left => app.connect_left(),
            KeyCode::Right => app.connect_right(),
            KeyCode::Backspace => app.connect_backspace(),
            KeyCode::Enter => match app.connect_submit() {
                Ok(settings) => submit_connect(app, rt, &settings),
                Err(_) => {} // error is shown inside the form
            },
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => {
                app.connect_form = None;
            }
            // Ctrl+D fills the default OpenCode URL + model; only the key
            // is left to type. (A bare `d` would be ambiguous with typing.)
            KeyCode::Char('d') | KeyCode::Char('D') if mods.contains(KeyModifiers::CONTROL) => {
                app.connect_fill_defaults();
            }
            KeyCode::Char(c) => {
                if !mods.contains(KeyModifiers::CONTROL)
                    && !mods.contains(KeyModifiers::ALT)
                {
                    app.connect_insert(c);
                }
            }
            _ => {}
        }
        return false;
    }

    // --- `/` autocomplete palette is open: nav keys go to the palette ---
    if !app.slash_matches().is_empty() {
        match code {
            KeyCode::Up => {
                app.slash_prev();
                return false;
            }
            KeyCode::Down => {
                app.slash_next();
                return false;
            }
            KeyCode::Tab => {
                app.slash_accept();
                return false;
            }
            KeyCode::Esc => {
                app.slash_dismiss();
                return false;
            }
            KeyCode::Char('n') if mods.contains(KeyModifiers::CONTROL) => {
                app.slash_next();
                return false;
            }
            KeyCode::Char('p') if mods.contains(KeyModifiers::CONTROL) => {
                app.slash_prev();
                return false;
            }
            _ => {}
        }
    }

    match code {
        KeyCode::Esc => {
            // Emergency brake: force-stop outstanding work. Esc never
            // kills the TUI itself — Ctrl+C / /exit do that.
            if rt.running {
                force_stop(app, rt);
            }
        }
        KeyCode::Char('d') if mods.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
            return true;
        }
        KeyCode::Up => app.history_prev(),
        KeyCode::Down => app.history_next(),
        KeyCode::PageUp => app.scroll_up(10),
        KeyCode::PageDown => app.scroll_down(10),
        KeyCode::Left => app.move_cursor_left(),
        KeyCode::Right => app.move_cursor_right(),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Enter if mods.contains(KeyModifiers::SHIFT) || mods.contains(KeyModifiers::ALT) => {
            app.insert_newline();
        }
        KeyCode::Enter => {
            // Palette open: Enter runs the highlighted command, not the
            // half-typed token (`/mo` -> `/models`, never "Unknown command").
            app.slash_confirm();
            let raw = app.take_input();
            if raw.trim().is_empty() {
                return false;
            }
            // `?` alone opens help.
            if raw.trim() == "?" {
                app.show_help = true;
                return false;
            }
            match app.interpret_input(raw.clone()) {
                SlashAction::Quit => return true,
                SlashAction::Local(msg) => {
                    if !msg.is_empty() && msg != "help" {
                        app.transcript.push(anvil_tui::app::TranscriptEntry::System(msg));
                        persist(app);
                    }
                }
                SlashAction::GitDiff => {
                    let tx = agent_tx.clone();
                    let workspace = app.workspace.clone();
                    app.begin_background();
                    tokio::spawn(async move {
                        let msg = git_diff_text(&workspace).await;
                        let _ = tx.send(AgentEvent::Note(msg));
                    });
                }
                SlashAction::Send(text) => {
                    if rt.running {
                        app.transcript.push(anvil_tui::app::TranscriptEntry::System(
                            "Already working — Esc to cancel first.".to_string(),
                        ));
                        return false;
                    }
                    if config::api_key().is_empty() {
                        app.transcript.push(anvil_tui::app::TranscriptEntry::System(
                            "No API key configured — set ANVIL_API_KEY (or OPENCODE_ZEN_API_KEY) or use /connect.".to_string(),
                        ));
                        return false;
                    }
                    app.push_user(&text);
                    persist(app);
                    spawn_agent_task(rt, agent_tx, text);
                }
            }
        }
        KeyCode::Char('?') if app.input.is_empty() => {
            app.show_help = true;
        }
        KeyCode::Char(c) => {
            // Ctrl combos already handled; plain typing.
            if !mods.contains(KeyModifiers::CONTROL) {
                app.insert_char(c);
            }
        }
        _ => {}
    }
    false
}

// ---------------------------------------------------------------------------
// Session persistence + `/connect` submit.
// ---------------------------------------------------------------------------

/// Best-effort snapshot of the transcript (SPEC §19). Failures are silent:
/// history must never break the interactive session.
fn persist(app: &App) {
    let _ = anvil_tui::history::save(&app.snapshot());
}

/// `/diff` body: `git status --short` + `git diff --stat` for the workspace,
/// truncated for the transcript. Runs off the UI thread via `GitDiff`.
async fn git_diff_text(workspace: &str) -> String {
    if workspace.trim().is_empty() {
        return "No workspace known — cannot run git diff.".to_string();
    }
    async fn run(dir: &str, args: &[&str]) -> Result<String, String> {
        let out = tokio::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .await
            .map_err(|e| format!("could not run git ({e})"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            return Err(if err.is_empty() {
                "git command failed".to_string()
            } else {
                err
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
    let status = match run(workspace, &["status", "--short"]).await {
        Ok(s) => s,
        Err(e) => return format!("git status failed: {e}"),
    };
    let diff = match run(workspace, &["diff", "--stat"]).await {
        Ok(d) => d,
        Err(e) => return format!("git diff failed: {e}"),
    };
    const MAX_LINES: usize = 40;
    let mut lines: Vec<String> = status
        .lines()
        .chain(diff.lines())
        .map(|l| l.to_string())
        .collect();
    if lines.iter().all(|l| l.trim().is_empty()) {
        return "Working tree clean — no changes.".to_string();
    }
    lines.retain(|l| !l.trim().is_empty());
    let total = lines.len();
    lines.truncate(MAX_LINES);
    let mut out = lines.join("\n");
    if total > MAX_LINES {
        out.push_str(&format!("\n… and {} more lines", total - MAX_LINES));
    }
    out
}

/// Apply validated `/connect` settings: URL + model + API key persist to
/// the local config file (owner-only permissions); the live session picks
/// them up immediately via env + provider hot-swap.
fn submit_connect(app: &mut App, rt: &mut Runtime, s: &ConnectSettings) {
    app.apply_connect(s);
    let mut lines = vec![format!(
        "Connected: `{}` via {} (+ /chat/completions)",
        s.model, s.base_url
    )];
    match config::save_provider(&s.model, &s.base_url) {
        Ok(path) => lines.push(format!("Saved URL + model to {}", path.display())),
        Err(e) => lines.push(format!("Note: could not write config file ({e})")),
    }
    if !s.api_key.is_empty() {
        std::env::set_var("ANVIL_API_KEY", &s.api_key);
        match config::save_api_key(&s.api_key) {
            Ok(path) => lines.push(format!(
                "API key saved to {} (owner-only permissions) — no need to re-enter it next time.",
                path.display()
            )),
            Err(e) => lines.push(format!(
                "API key set for this session only (could not save: {e})."
            )),
        }
    } else {
        // Empty field clears any previously stored key.
        match config::save_api_key("") {
            Ok(_) => lines.push(
                "Cleared the stored API key — using ANVIL_API_KEY / OPENCODE_ZEN_API_KEY from the environment.".to_string(),
            ),
            Err(e) => lines.push(format!("Note: could not update stored key ({e}).")),
        }
    }
    let mut overridden = Vec::new();
    if std::env::var("ANVIL_MODEL").map(|v| !v.trim().is_empty()).unwrap_or(false) {
        overridden.push("ANVIL_MODEL");
    }
    if ["ANVIL_BASE_URL", "OPENCODE_BASE_URL"]
        .iter()
        .any(|k| std::env::var(k).map(|v| !v.trim().is_empty()).unwrap_or(false))
    {
        overridden.push("ANVIL_BASE_URL/OPENCODE_BASE_URL");
    }
    if !overridden.is_empty() {
        lines.push(format!(
            "Warning: {} env var(s) override the saved file until unset.",
            overridden.join(", ")
        ));
    }
    for line in lines {
        app.transcript
            .push(anvil_tui::app::TranscriptEntry::System(line));
    }
    // Hot-swap the connection (reads the just-set ANVIL_API_KEY from env)
    // and rebuild the agent around the new model id. In-flight runs keep
    // the old Arc<Agent> and finish undisturbed.
    rt.swapper.swap(HotSwapProvider::build_connection());
    let old = rt.agent.clone();
    rt.agent = Arc::new(Agent {
        config: AgentConfig {
            model: s.model.clone(),
            max_iterations: old.config.max_iterations,
            context_max_tokens: old.config.context_max_tokens,
            temperature: old.config.temperature,
        },
        provider: Arc::new(rt.swapper.clone()) as Arc<dyn anvil_model::ModelProvider>,
        tools: old.tools.clone(),
        tool_ctx: old.tool_ctx.clone(),
        permissions: std::sync::Mutex::new(
            old.permissions.lock().map(|p| p.clone()).unwrap_or_else(|_| anvil_core::PermissionSet::defaults()),
        ),
        skills: old.skills.clone(),
        workspace: old.workspace.clone(),
    });
    app.pin_to_bottom();
    persist(app);
}

// ---------------------------------------------------------------------------
// Headless run + project init (SPEC §4).
// ---------------------------------------------------------------------------

/// `anvil init`: project config skeleton + instruction template.
fn cmd_init() -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let dir = cwd.join(".anvil");
    std::fs::create_dir_all(dir.join("skills"))?;
    let cfg_path = dir.join("config.yaml");
    if cfg_path.exists() {
        println!("{} exists — left alone.", cfg_path.display());
    } else {
        let cfg = config::load();
        std::fs::write(
            &cfg_path,
            format!(
                "# Anvil project config — overrides ~/.config/anvil/config.yaml\n\
                 model: \"{}\"\n\
                 base_url: \"{}\"\n\
                 \n\
                 # permissions: allow | ask | deny\n\
                 #permissions:\n\
                 #  read: allow\n\
                 #  shell: ask\n\
                 \n\
                 # MCP servers (SPEC §15):\n\
                 #mcp:\n\
                 #  servers:\n\
                 #    docs:\n\
                 #      command: npx\n\
                 #      args: [\"-y\", \"@modelcontextprotocol/server-docs\"]\n",
                cfg.model, cfg.base_url,
            ),
        )?;
        println!("wrote {}", cfg_path.display());
    }
    let md = cwd.join("ANVIL.md");
    if md.exists() {
        println!("{} exists — left alone.", md.display());
    } else {
        std::fs::write(
            &md,
            "# Project instructions\n\n\
             Anvil reads this file (plus `.anvil/AGENTS.md` and nested \
             `ANVIL.md` files) into every task. Describe conventions, \
             how to build/test, and what \"done\" means.\n",
        )?;
        println!("wrote {}", md.display());
    }
    Ok(())
}

/// `anvil run`: one headless agent task, streaming to stdout.
/// `--yes` auto-allows approval-gated tools; otherwise they are denied
/// with a note (never silently executed — SPEC §28).
async fn cmd_run(prompt: String, yes: bool) -> anyhow::Result<()> {
    if config::api_key().is_empty() {
        anyhow::bail!("no API key — set ANVIL_API_KEY (or OPENCODE_ZEN_API_KEY)");
    }
    let BuiltAgent { agent, warnings, .. } = build_agent().await?;
    for w in warnings {
        eprintln!("warning: {w}");
    }
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<AgentEvent>();
    let (ap_tx, mut ap_rx) = mpsc::unbounded_channel::<ApprovalJob>();
    let (cancel_tx, cancel_rx) = watch::channel(false);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = cancel_tx.send(true);
    });
    let gate = ApprovalGate::new(ap_tx);
    let task = tokio::spawn(async move {
        let mut history = Vec::new();
        agent.run(prompt, &mut history, ev_tx, cancel_rx, gate).await
    });
    use std::io::Write as _;
    loop {
        tokio::select! {
            biased;
            job = ap_rx.recv() => {
                let Some(job) = job else { break };
                eprintln!("\napproval required: {}", job.request.preview);
                let d = if yes {
                    eprintln!("[--yes] allowed once.");
                    ApprovalDecision::AllowOnce
                } else {
                    eprintln!("denied (re-run with --yes to allow).");
                    ApprovalDecision::Deny
                };
                let _ = job.reply.send(d);
            }
            ev = ev_rx.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    AgentEvent::Thinking => eprintln!("thinking…"),
                    AgentEvent::TextDelta(d) => {
                        print!("{d}");
                        let _ = std::io::stdout().flush();
                    }
                    AgentEvent::TextDone => println!(),
                    AgentEvent::ToolStarted(c) => println!("\n> {} {}", c.name, c.summary),
                    AgentEvent::ToolOutput(r) => {
                        let mark = if r.success { "✓" } else { "✗" };
                        println!("  {mark} {}", r.summary);
                    }
                    AgentEvent::ApprovalRequired(_) => {}
                    AgentEvent::Usage { status } => eprintln!("{status}"),
                    AgentEvent::Note(m) => println!("{m}"),
                    AgentEvent::Error(e) => {
                        eprintln!("\nerror: {e}");
                        break;
                    }
                    AgentEvent::Completed | AgentEvent::Cancelled => break,
                }
            }
        }
    }
    match task.await? {
        Ok(o) => {
            eprintln!("done in {} iterations, {} tool calls.", o.iterations, o.tool_calls);
            Ok(())
        }
        Err(anvil_core::AgentError::Cancelled) => {
            eprintln!("cancelled.");
            Ok(())
        }
        Err(e) => anyhow::bail!("{e}"),
    }
}
