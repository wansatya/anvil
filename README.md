# ANVIL

```text
 █████╗   ███╗   ██╗  ██╗   ██╗  ██╗  ██╗
██╔══██╗  ████╗  ██║  ██║   ██║  ██║  ██║
███████║  ██╔██╗ ██║  ██║   ██║  ██║  ██║
██╔══██║  ██║╚██╗██║  ╚██╗ ██╔╝  ██║  ██║
██║  ██║  ██║ ╚████║   ╚████╔╝   ██║  ███████╗
╚═╝  ╚═╝  ╚═╝  ╚═══╝    ╚═══╝    ╚═╝  ╚══════╝
```

> A terminal-native AI engineering agent. Not a chatbot, not a bare LLM
> client — Anvil combines project context, skills, tools, MCP servers, model
> routing, and an agent execution loop behind one fast TUI.
>
> The model generates intelligence. **Anvil provides the environment in which
> that intelligence can act.**

Status: MVP (Phase 1 — workspace, CLI, config, TUI, event loop).
Primary language: Rust. Target: Linux / macOS.

---

## Quickstart

```bash
cd my-project
anvil
```

Then type:

```text
Fix the failing tests.
```

Anvil inspects the repo, reads the relevant files, shows its plan, asks
permission before anything destructive, edits, runs tests, iterates, and
summarizes the result.

---

## Install

No manual setup needed: `./install.sh` installs missing support libraries
first (forced, per OS), with a spinner on long steps and the full log kept
on failure (`/tmp/anvil-install-<pid>.log`).

| OS | Auto-installed when missing |
| -- | --------------------------- |
| Linux | `git curl tar` + C toolchain (`build-essential`/`base-devel`/etc. via apt, dnf, yum, pacman, apk, zypper with sudo) + Rust via rustup |
| macOS | Xcode Command Line Tools (git + compiler) + Rust via rustup |
| Windows (Git Bash) | Git + Rust via winget |

Skip it with `./install.sh --no-deps` (or `ANVIL_NO_DEPS=1`) if you manage
toolchains yourself. Every install first removes any previously installed
`anvil` binary (including a stale opposite-platform counterpart), so a
failed build can never leave an old binary behind — the rest of the install
directory is left untouched.

**From a local checkout** (builds `--release`, installs to `~/.local/bin`):

```bash
./install.sh
./install.sh --prefix ~/.local/bin
```

**Via curl** (once this repo is on GitHub — replace `owner/repo`):

```bash
curl -fsSL https://raw.githubusercontent.com/wansatya/anvil/main/install.sh | bash
curl -fsSL https://raw.githubusercontent.com/wansatya/anvil/main/install.sh | bash -s -- --prefix ~/.local/bin
```

Useful overrides:

```bash
ANVIL_REPO=wansatya/anvil ./install.sh        # remote source build from GitHub
ANVIL_REF=main ./install.sh               # git ref for remote builds
PREFIX=/usr/local/bin ./install.sh        # system-wide install
```

The installer prefers a prebuilt GitHub release asset
(`anvil-<os>-<arch>.tar.gz`) when `ANVIL_REPO` is set, falls back to a
`cargo build --release` from cloned sources, then verifies with
`anvil version` and warns if the install dir is not on `PATH`.

---

## Model

Anvil is model-agnostic and talks to any OpenAI-compatible
`POST /v1/chat/completions` provider. The out-of-the-box default is
**Muse Spark 1.3 (free) via the OpenCode Zen gateway**:

| Setting   | Default                                        |
| --------- | ---------------------------------------------- |
| model     | `opencode/muse-spark-1.3-contributor-free`     |
| base URL  | `https://opencode.ai/zen/v1` (+ `/chat/completions`) |

```bash
export ANVIL_API_KEY=xxx        # or OPENCODE_ZEN_API_KEY / OPENCODE_API_KEY
export ANVIL_MODEL=opencode/muse-spark-1.3-contributor-free
export ANVIL_BASE_URL=https://opencode.ai/zen/v1
anvil
```

`anvil doctor` shows the effective endpoint and whether a key is set
(redacted, never printed). Precedence: `ANVIL_*` env vars beat
`~/.config/anvil/config.yaml` (override path with `ANVIL_CONFIG`), which
beats built-in defaults. Future phases add more providers behind the same
`ModelProvider` trait — the agent runtime never sees provider-specific types.

### `/connect`

Inside the TUI, `/connect` opens a form to set the provider **URL**,
**model**, and **API key** without leaving the terminal:

- `Tab` switches fields, `Enter` saves, `Esc` cancels.
- URL + model + API key all persist to `~/.config/anvil/config.yaml`, so you
  only enter the key once. The file (and folders Anvil creates for it) is
  locked to owner-only permissions (`0600`/`0700` on Unix), and the key is
  never printed, logged, or saved in session transcripts. Leaving the key
  field empty clears the stored key.
- If `ANVIL_API_KEY` / `ANVIL_MODEL` / `ANVIL_BASE_URL` are set in the
  environment, they keep overriding the file until unset (Anvil warns you).

---

## CLI

```bash
anvil                 # start the interactive TUI (default)
anvil init            # initialize .anvil/config.yaml in this project
anvil run "<prompt>"  # headless single task (streams to stdout; --yes to auto-allow tools)
anvil models          # list configured models
anvil skills          # list available skills (Phase 6)
anvil mcp             # list configured MCP servers (Phase 7)
anvil config          # show effective configuration
anvil doctor          # check config, provider, and tool availability
anvil version         # show version
```

---

## TUI

Built on `ratatui` + `crossterm` — no custom rendering engine.

```text
┌──────────────────────────────────────────────────────────────┐
│ ANVIL                                      model: muse-spark │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  User                                                       │
│  Fix the failing CAN parser tests.                          │
│                                                              │
│  Anvil                                                      │
│  I found 3 failing tests. I'll inspect the parser and       │
│  related test fixtures first.                               │
│                                                              │
│  > anvil.read_file src/can/parser.cpp                       │
│    ✓ 312 lines                                              │
│                                                              │
│  > anvil.shell cargo test                                   │
│    ✗ 3 tests failed                                          │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│ > Type a message...                                         │
├──────────────────────────────────────────────────────────────┤
│ ↑↓ scroll   Enter send   / commands   Esc stop   Ctrl+C exit   ? help │
└──────────────────────────────────────────────────────────────┘
```

Streaming responses render live with a `▌` cursor; tool calls and results
are visualized inline; dangerous tools pop an approval dialog:

```text
┌─────────────────────────────────────────────┐
│ ANVIL wants to execute:                     │
│                                             │
│ cargo test                                  │
│                                             │
│ [Enter] Allow    [a] Always    [Esc] Deny   │
└─────────────────────────────────────────────┘
```

**Keys**

| Key                    | Action                                     |
| ---------------------- | ------------------------------------------ |
| `Enter`                | send message                               |
| `Shift+Enter` / `Alt+Enter` | newline in input (multiline)          |
| `↑` `↓` / `PgUp` `PgDn` | scroll transcript                         |
| `Esc`                  | force-stop running work (aborts the task; never kills the TUI) |
| `Ctrl+C`               | exit the TUI (aborts any running work first) |
| `Ctrl+D`               | quit                                       |
| `?`                    | toggle help                                |
| `Tab` / `↑` `↓`        | complete / navigate the `/` palette        |

**Slash commands** (handled locally, never sent to the model).
Type `/` to open the autocomplete palette (`Tab` completes, `Enter` runs,
`Esc` dismisses):

```text
/help /clear /new /connect /history /model /models /skills /mcp
/context /diff /status /compact /debug /exit
```

Sessions auto-save to `~/.local/share/anvil/sessions/` (override with
`ANVIL_DATA_DIR`); `/history` lists saved sessions, `/new` starts a fresh
one, and `/status` / `/context` / `/model` report live session state.

### Seeing under the hood

Like other coding CLIs, Anvil narrates its work: the header shows a live
`thinking · Ns` elapsed timer plus per-iteration status (`iter 2/50 ·
running 2 tool(s)… · 1.2k tokens`), tool calls stream into the transcript,
and `/debug` opens a raw event feed pane (every model/tool/approval event
with errors in full). Timeouts guard every network hop so a hung provider
fails loudly instead of spinning forever: 15s connect, 60s to first
response, 120s max mid-stream silence.

Troubleshooting a stuck or empty reply:

| Symptom | Likely cause |
| ------- | ------------ |
| `HTTP 401/403` | wrong or missing API key for this provider |
| `HTTP 404` | model id doesn't exist at this base URL (e.g. Responses-only model on `/chat/completions`) |
| `HTTP 429` | rate limited — wait and retry |
| `provider did not answer within 60s` | wrong base URL / no network |
| `no data for 120s` | provider stalled mid-stream — Esc and retry |

## Skills

Anvil ships with a built-in ESP32 starter pack for C/C++ developers doing
physical AI — embedded in the binary, always available, no install needed:

| Skill | Covers |
| ----- | ------ |
| `esp32-start` | series picker (ESP32/S3/C3/C6/H2), toolchain setup, flash & monitor |
| `esp-idf-patterns` | `app_main`, FreeRTOS tasks, logging, `esp_err_t`, Kconfig, NVS |
| `esp32-gpio-adc-pwm` | GPIO, calibrated ADC, LEDC, RMT + hardware pitfalls |
| `esp32-buses` | I2C, SPI, UART wiring and drivers |
| `esp32-wifi-mqtt` | reconnecting WiFi, MQTT over TLS, offline buffering |
| `esp32-power-sleep` | sleep modes, ULP, battery measurement, power budgets |
| `esp32-ota-debug` | OTA with rollback, partitions, JTAG, coredumps, watchdog |
| `esp32-edge-ai` | sensor DSP, ESP-DL/TinyML deployment, quantization |
| `esp32-safety` | mains relays, level shifting, LiPo rules, ESD |

`/skills` (or `anvil skills`) lists built-ins alongside project
(`.anvil/skills/`) and global (`~/.config/anvil/skills/`) skills — a
project/global skill with the same name shadows the built-in. The model
sees only names + descriptions; the full text loads on demand when the
agent emits `use skill <name>`.

The TUI stays responsive because all long-running work is async: the agent
runtime emits `AgentEvent`s (`Thinking`, `TextDelta`, `ToolStarted`,
`ToolOutput`, `ApprovalRequired`, `Completed`, `Error`, …) over a channel,
and user input flows back as `UserEvent`s. The runtime is fully testable
without a terminal.

---

## Project layout

```text
anvil/
├── install.sh            # release build + local install (+ curl support)
├── deploy.sh             # cut a release: test, tag, push (CI builds binaries)
├── .github/workflows/release.yml  # builds + uploads release binaries
├── SPEC.md               # full product specification (36 sections)
├── Cargo.toml            # Rust workspace
├── crates/
│   ├── anvil-cli/        # `anvil` binary: commands + TUI event loop
│   ├── anvil-core/       # agent loop, context, permissions, events
│   ├── anvil-model/      # provider trait + OpenAI-compatible streaming
│   ├── anvil-tools/      # read/write/edit/list/search/shell/git
│   ├── anvil-mcp/        # MCP stdio client + registry adapters
│   ├── anvil-skills/     # skill discovery + built-in ESP32 pack
│   └── anvil-tui/        # Ratatui App state + rendering
└── tests/
```

---

## Roadmap

Implemented in order — no vector DB, no multi-agent framework, no cloud
account until the local loop works:

1. **Skeleton** (done): workspace, CLI, config, TUI, event loop
2. **Model** (done): OpenAI-compatible streaming provider, message types
3. **Agent** (done): loop, tool calls, events, cancellation
4. **Tools** (done): read/list/search/write/edit/shell/git_status/git_diff
5. **Context** (done): workspace discovery, `ANVIL.md`, git state, token budget
6. **Skills** (done): `SKILL.md` parsing, lazy loading, built-in ESP32 pack
7. **MCP** (done): server lifecycle, tool discovery/execution
8. **Sessions** (done): transcript snapshots, `/history`, `/new`,
   `anvil --session <id>` resume, slash commands
9. **Hardening**: permissions, security, error handling, tests, packaging
   (typed errors + permission engine done; config surface partial)

Remaining: full permission YAML surface, secret redaction pass, and the
acceptance run against a live provider.

Acceptance: `cargo build --release` produces `anvil`; `./target/release/anvil`
opens the TUI; pointing it at a repo with failing tests ends with a fix,
a green test run, and a summary.

---

## Development

```bash
cargo build                  # debug build
cargo test                   # unit + integration tests (mocked providers only)
cargo build --release        # release binary at target/release/anvil
RUST_LOG=debug cargo run -p anvil-cli   # debug logs (stderr, never the TUI)
```

Tests never require a real API key. See `SPEC.md` (§33) for the required
unit/integration coverage and (§28) for the security rules: workspace
sandboxing, approval for writes/shell, no secret logging.

---

## Releasing

`target/` is git-ignored and never pushed — it's gigabytes of intermediates
that would blow past GitHub's limits. Instead, release binaries are built
**on GitHub's machines** and attached to a GitHub Release, which is exactly
what `install.sh` downloads:

```bash
./deploy.sh            # tag v<version from Cargo.toml>, push the tag
./deploy.sh v0.2.0     # explicit tag
./deploy.sh --dry-run  # show what would happen
```

`deploy.sh` requires a clean tree, then syncs the repo version to the tag:
passing `v0.2.0` bumps `version` in every `crates/*/Cargo.toml` (+ `Cargo.lock`)
and commits it as `release v0.2.0`, so the TUI footer, `anvil version`, and the
git tag always agree. Omitting the tag reuses the Cargo version with no bump.
Tests gate the release; then the branch (if a bump was committed) and the tag
are pushed.
`.github/workflows/release.yml` takes it from there: tests + release build
per platform (`linux-x86_64`, `macos-aarch64`, `windows-x86_64`), and uploads
`anvil-<platform>.tar.gz`, the raw binary, and checksums to the release.

Source stays in git (kilobytes); only the built binaries ship to users.

---

## License

TBD — add a `LICENSE` file before publishing releases.
