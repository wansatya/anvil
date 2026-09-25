# `spec.md`

````md
# ANVIL — Engineering Agent CLI

> An open, terminal-native engineering agent runtime combining context, skills,
> tools, MCP, model routing, and an OpenAI-compatible inference API.

Command: `anvil`

Status: MVP specification
Target: Linux / macOS first
Primary language: Rust

---

## 1. Product Definition

Anvil is a terminal-native AI engineering agent.

It is NOT merely a chatbot and NOT merely an LLM API client.

Anvil combines:

1. Project context
2. Persistent instructions
3. Skills
4. Tools
5. MCP servers
6. Agent execution loop
7. Model routing
8. Conversation/session state
9. Human approval
10. Streaming model responses

The user interacts primarily through a TUI.

Example:

```bash
anvil
````

Anvil starts in the current directory and understands the project.

The user can then say:

```text
Fix the failing tests.
```

Anvil should:

1. inspect the repository
2. understand relevant files
3. inspect git state
4. determine what needs to change
5. explain its plan
6. request approval when required
7. edit files
8. run tests
9. inspect failures
10. iterate
11. summarize the result

---

# 2. Core Philosophy

## 2.1 Model is replaceable

Anvil must NOT be tightly coupled to a particular LLM.

Models are providers.

The agent runtime is the product.

Supported providers should include:

* OpenAI-compatible HTTP APIs
* local models
* future native providers

Primary interface:

```text
POST /v1/chat/completions
```

Anvil must be able to consume any provider implementing the OpenAI-compatible
Chat Completions API.

---

## 2.2 Context is compiled

Never blindly send the entire repository to the model.

Anvil builds a context package from:

```text
user request
+
project instructions
+
relevant files
+
git state
+
conversation
+
skills
+
tool results
+
MCP results
```

The context layer should be modular so a future Context Compiler can replace
the initial implementation.

---

## 2.3 Tools are first-class

The model does not directly modify the computer.

It requests tool calls.

Anvil executes the tools.

Example:

```text
Model
  ↓
tool_call: read_file
  ↓
Anvil
  ↓
filesystem
  ↓
tool_result
  ↓
Model
```

This separation is mandatory.

---

# 3. MVP Goals

The MVP must support:

* interactive TUI
* streaming responses
* OpenAI-compatible model providers
* project discovery
* project instructions
* file reading
* file searching
* file writing
* file editing
* shell execution
* git inspection
* git diff
* tool approval
* session history
* skills
* MCP servers
* model configuration
* basic agent loop

The MVP does NOT need:

* GUI
* IDE plugin
* cloud account
* multi-user collaboration
* autonomous background agents
* model training
* vector database
* complex distributed execution
* telemetry by default

---

# 4. CLI

Binary:

```bash
anvil
```

## Commands

```bash
anvil
```

Start interactive TUI.

```bash
anvil init
```

Initialize Anvil configuration in the current project.

```bash
anvil run "<prompt>"
```

Execute a task without entering the interactive TUI.

Example:

```bash
anvil run "Fix the failing tests"
```

```bash
anvil models
```

List configured models.

```bash
anvil skills
```

List available skills.

```bash
anvil mcp
```

List configured MCP servers.

```bash
anvil config
```

Show effective configuration.

```bash
anvil doctor
```

Check configuration, provider connectivity and tool availability.

```bash
anvil version
```

Show version.

---

# 5. TUI

Use a mature Rust terminal UI framework.

Preferred:

```text
ratatui
crossterm
```

Do NOT build a custom terminal rendering engine.

## Layout

Default layout:

```text
┌──────────────────────────────────────────────────────────────┐
│ ANVIL                                      model: cpp-agent   │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  User                                                       │
│  Fix the failing CAN parser tests.                          │
│                                                              │
│  Anvil                                                      │
│  I found 3 failing tests. I'll inspect the parser and       │
│  related test fixtures first.                               │
│                                                              │
│  > read_file src/can/parser.cpp                             │
│    ✓ 312 lines                                              │
│                                                              │
│  > run: cargo test                                           │
│    ✗ 3 tests failed                                          │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│ > Type a message...                                         │
├──────────────────────────────────────────────────────────────┤
│ ↑↓ scroll   Enter send   Ctrl+C cancel   ? help             │
└──────────────────────────────────────────────────────────────┘
```

## TUI requirements

Support:

* scrolling
* multiline input
* streaming text
* tool-call visualization
* tool results
* errors
* approval prompts
* status indicators
* cancellation
* session history

The TUI must remain responsive while model requests and tools execute.

All long-running work must be asynchronous.

---

# 6. Agent Loop

The core runtime is an iterative agent loop.

Pseudo-flow:

```text
USER MESSAGE
     ↓
BUILD CONTEXT
     ↓
CALL MODEL
     ↓
STREAM RESPONSE
     ↓
MODEL REQUESTS TOOL?
     │
   ┌─┴─┐
  YES  NO
   │    │
EXECUTE  COMPLETE
TOOL
   │
   ↓
TOOL RESULT
   │
   └──────────────→ CALL MODEL AGAIN
```

Maximum iteration count must be configurable.

Default:

```yaml
agent:
  max_iterations: 50
```

The loop must terminate when:

* model produces a final response
* user cancels
* maximum iterations reached
* unrecoverable error occurs

---

# 7. Model Interface

Create a provider abstraction.

Conceptual Rust trait:

```rust
#[async_trait]
pub trait ModelProvider {
    async fn chat(
        &self,
        request: ChatRequest,
    ) -> Result<ChatStream, ModelError>;
}
```

Do not expose provider-specific types throughout the application.

Normalize provider responses into internal types.

---

# 8. OpenAI-Compatible Provider

Implement first.

Configuration:

```yaml
models:
  default: cpp-agent

  providers:
    local:
      type: openai-compatible
      base_url: http://localhost:8000/v1
      api_key: local

    cloud:
      type: openai-compatible
      base_url: https://api.example.com/v1
      api_key_env: ANVIL_API_KEY

  models:
    cpp-agent:
      provider: cloud
      model: cpp-agent
```

The implementation must support:

```text
POST /v1/chat/completions
```

with:

```json
{
  "model": "cpp-agent",
  "messages": [],
  "stream": true,
  "tools": []
}
```

Support:

* streaming
* tool calls
* system messages
* user messages
* assistant messages
* tool messages
* temperature
* max tokens
* model selection

---

# 9. Internal Message Model

Create provider-independent types.

Example:

```rust
pub enum Message {
    System(String),
    User(String),
    Assistant(AssistantMessage),
    Tool(ToolMessage),
}

pub struct AssistantMessage {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

pub struct ToolMessage {
    pub tool_call_id: String,
    pub content: String,
}
```

Do not pass raw OpenAI JSON through the entire application.

---

# 10. Tool System

Every tool must have:

```text
name
description
JSON schema
execution function
permission policy
```

Example:

```rust
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    async fn execute(
        &self,
        arguments: serde_json::Value,
        context: ToolContext,
    ) -> Result<ToolResult, ToolError>;
}
```

---

# 11. Built-in Tools

MVP tools:

## read_file

```json
{
  "path": "src/main.rs"
}
```

Must:

* reject paths outside workspace
* return useful line information
* handle binary files safely
* truncate extremely large files

---

## write_file

```json
{
  "path": "src/main.rs",
  "content": "..."
}
```

Must require approval by default.

---

## edit_file

Preferred over repeatedly rewriting entire files.

Arguments:

```json
{
  "path": "src/main.rs",
  "old": "old text",
  "new": "new text"
}
```

If `old` does not match exactly, fail safely.

Never silently modify an unexpected region.

---

## list_files

List project files.

Support ignore rules.

Respect:

```text
.gitignore
```

---

## search

Search repository text.

Preferred implementation:

```text
ripgrep
```

if available.

Fallback to native Rust implementation.

---

## shell

Execute shell commands.

Example:

```json
{
  "command": "cargo test"
}
```

Shell execution is potentially destructive.

Default policy:

```text
ask
```

User may configure:

```yaml
permissions:
  shell: ask
```

Possible values:

```text
allow
ask
deny
```

---

## git_status

Return:

* branch
* modified files
* staged files
* untracked files

---

## git_diff

Return current diff.

---

# 12. Tool Approval

Dangerous tools must require explicit approval.

Example:

```text
┌─────────────────────────────────────────────┐
│ ANVIL wants to execute:                     │
│                                             │
│ cargo test                                  │
│                                             │
│ [Enter] Allow    [a] Always    [Esc] Deny   │
└─────────────────────────────────────────────┘
```

At minimum classify:

```text
read-only
safe-write
shell
destructive
network
```

Default:

```yaml
permissions:
  read: allow
  search: allow
  git_read: allow
  write: ask
  shell: ask
  destructive: ask
  network: ask
```

---

# 13. Project Context

When starting:

```bash
anvil
```

Anvil discovers the workspace.

Workspace root should normally be determined from:

```text
.git
```

or current directory.

Load project instructions from:

```text
ANVIL.md
```

Also support:

```text
.anvil/AGENTS.md
```

Instructions are hierarchical.

Example:

```text
project/
├── ANVIL.md
├── src/
│   ├── ANVIL.md
│   └── parser/
│       └── ANVIL.md
```

When operating on:

```text
src/parser/foo.cpp
```

the applicable instructions are:

```text
root ANVIL.md
+
src/ANVIL.md
+
src/parser/ANVIL.md
```

More specific instructions override general instructions.

---

# 14. Skills

Skills are reusable domain capabilities.

Directory:

```text
.anvil/skills/
```

Example:

```text
.anvil/
└── skills/
    └── cpp-review/
        └── SKILL.md
```

Global skills:

```text
~/.config/anvil/skills/
```

Skill format:

```md
---
name: cpp-review
description: Review C++ code for correctness, safety and performance.
---

# C++ Review

When reviewing C++:

- inspect ownership
- inspect lifetime
- inspect concurrency
- inspect error handling
- inspect undefined behavior
- inspect performance
```

The model initially receives:

```text
skill name
skill description
```

The full skill is loaded only when selected/relevant.

Do not load every skill into every request.

---

# 15. MCP

Anvil must support Model Context Protocol servers.

Configuration:

```yaml
mcp:
  servers:
    embedded-docs:
      command: uv
      args:
        - run
        - embedded_mcp.py

    github:
      command: npx
      args:
        - -y
        - "@modelcontextprotocol/server-github"
```

Anvil should:

1. start configured MCP servers
2. discover tools
3. normalize MCP tools into the Anvil tool registry
4. expose them to the model
5. execute requested tools
6. return results to the model

MCP tools should look identical to native tools from the agent's perspective.

---

# 16. Tool Namespace

Avoid collisions.

Native:

```text
anvil.read_file
anvil.write_file
anvil.search
anvil.shell
```

MCP:

```text
mcp.embedded_docs.search
mcp.github.create_issue
```

Internal model-facing names may be flattened if required by the provider.

---

# 17. Context Builder

Create a dedicated module:

```text
src/context/
├── mod.rs
├── builder.rs
├── project.rs
├── instructions.rs
├── files.rs
├── git.rs
└── budget.rs
```

ContextBuilder should produce:

```rust
pub struct CompiledContext {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub estimated_tokens: usize,
}
```

The context builder must have a token budget.

Example:

```yaml
context:
  max_tokens: 32000
```

Priority:

```text
1. system instructions
2. user request
3. relevant project instructions
4. active skill
5. recent tool results
6. relevant source files
7. git state
8. older conversation
```

Never exceed the configured budget.

---

# 18. Repository Relevance

MVP relevance can be simple.

Use:

1. explicit file references from user
2. recently touched files
3. search results
4. imports/includes
5. git diff
6. directory proximity

Do NOT implement a vector database in MVP.

Architecture should allow a future semantic index.

---

# 19. Session System

Sessions are persistent conversations.

Storage:

```text
~/.local/share/anvil/sessions/
```

or platform equivalent.

Each session should have:

```text
session_id
workspace
created_at
updated_at
messages
tool_calls
model
metadata
```

Commands:

```text
/new
/history
/clear
/exit
```

Optional CLI:

```bash
anvil --session <id>
```

---

# 20. Slash Commands

Inside TUI:

```text
/help
/clear
/new
/model
/models
/skills
/mcp
/context
/diff
/status
/compact
/exit
```

Slash commands are handled by Anvil itself.

Do not send them to the model.

---

# 21. Streaming

Model responses must stream into the TUI.

Do not wait for the complete response.

Rendering should support:

```text
▌
```

cursor while streaming.

Tool calls should be displayed separately.

Example:

```text
Anvil

I'll inspect the parser first.

> search("CAN_TIMEOUT")

✓ 7 matches

> read_file("src/can/parser.cpp")

✓ 312 lines

I found the issue...
```

---

# 22. Cancellation

The user must be able to cancel:

```text
Ctrl+C
```

Cancellation must propagate to:

```text
TUI
 ↓
agent loop
 ↓
model request
 ↓
tool execution
```

Do not leave background tasks running after cancellation.

---

# 23. Configuration

Default location:

```text
~/.config/anvil/config.yaml
```

Project configuration:

```text
.anvil/config.yaml
```

Project config overrides global config.

Example:

```yaml
model: cpp-agent

agent:
  max_iterations: 50

context:
  max_tokens: 32000

permissions:
  write: ask
  shell: ask
  network: ask

models:
  providers:
    default:
      type: openai-compatible
      base_url: https://api.example.com/v1
      api_key_env: ANVIL_API_KEY

  models:
    cpp-agent:
      provider: default
      model: cpp-agent
```

Environment variables must override config values.

---

# 24. Environment Variables

Support:

```text
ANVIL_API_KEY
ANVIL_BASE_URL
ANVIL_MODEL
ANVIL_CONFIG
```

Example:

```bash
export ANVIL_BASE_URL=https://api.example.com/v1
export ANVIL_API_KEY=xxx
export ANVIL_MODEL=cpp-agent
anvil
```

---

# 25. Architecture

Recommended Rust workspace:

```text
anvil/
├── Cargo.toml
├── README.md
├── spec.md
│
├── crates/
│   ├── anvil-cli/
│   │   └── src/
│   │
│   ├── anvil-core/
│   │   └── src/
│   │       ├── agent/
│   │       ├── context/
│   │       ├── session/
│   │       ├── permissions/
│   │       └── events/
│   │
│   ├── anvil-model/
│   │   └── src/
│   │       ├── provider.rs
│   │       ├── openai.rs
│   │       └── types.rs
│   │
│   ├── anvil-tools/
│   │   └── src/
│   │       ├── filesystem.rs
│   │       ├── shell.rs
│   │       ├── search.rs
│   │       └── git.rs
│   │
│   ├── anvil-mcp/
│   │   └── src/
│   │
│   ├── anvil-skills/
│   │   └── src/
│   │
│   └── anvil-tui/
│       └── src/
│           ├── app.rs
│           ├── ui.rs
│           ├── input.rs
│           └── events.rs
│
└── tests/
```

---

# 26. Event Architecture

The TUI must not directly control the agent runtime.

Use an event-driven architecture.

Example:

```rust
pub enum AgentEvent {
    Thinking,
    TextDelta(String),
    ToolStarted(ToolCall),
    ToolOutput(ToolResult),
    ApprovalRequired(ApprovalRequest),
    Completed,
    Error(String),
}
```

Flow:

```text
Agent
  ↓
AgentEvent
  ↓
Event channel
  ↓
TUI
```

User input flows in the opposite direction:

```text
TUI
 ↓
UserEvent
 ↓
Agent
```

This keeps the runtime testable without the TUI.

---

# 27. Error Handling

Never panic during normal operation.

Errors must be typed.

Examples:

```text
ModelError
ToolError
ConfigError
McpError
ContextError
SessionError
PermissionError
```

Display human-readable messages in TUI.

Include technical details behind an expandable/debug mechanism where practical.

---

# 28. Security

MVP security requirements:

* workspace path sandboxing for filesystem tools
* explicit approval for writes
* explicit approval for shell commands
* never print API keys
* never persist API keys in session logs
* redact secrets from tool output where practical
* MCP processes must inherit only necessary environment variables
* shell commands must run with the user's permissions
* never execute model-generated commands silently when approval is required

---

# 29. Logging

Default:

```text
quiet
```

Debug mode:

```bash
RUST_LOG=debug anvil
```

Logs must go to stderr or a log file.

Do not pollute the TUI with internal logs.

Never log:

```text
API keys
authorization headers
credentials
```

---

# 30. Model Routing

Create an abstraction even if MVP only has one model.

```rust
pub trait ModelRouter {
    async fn select(
        &self,
        request: &AgentRequest,
    ) -> Result<ModelId, RouterError>;
}
```

Future routing criteria:

```text
task complexity
context size
latency
cost
model capability
local vs remote
language
tool-calling ability
```

Example future policy:

```text
simple task → cheap model
complex C++ task → specialist model
large context → long-context model
private repository → local model
```

Do not implement sophisticated routing in MVP.

---

# 31. Cost and Usage

The runtime should track:

```text
input tokens
output tokens
latency
model
tool calls
estimated cost
```

Internal event:

```rust
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms: u64,
}
```

Display optionally:

```text
model: cpp-agent
tokens: 8.2k
latency: 1.8s
```

---

# 32. API Compatibility

Anvil itself does NOT need to expose an HTTP API in MVP.

It consumes APIs.

However, design internal types so a future:

```text
anvil-server
```

can expose:

```text
/v1/chat/completions
```

without rewriting the agent runtime.

Future architecture:

```text
                 anvil-core
                /          \
               /            \
        anvil CLI        anvil-server
           │                   │
          TUI           /v1/chat/completions
```

---

# 33. Testing

Unit tests required for:

* configuration
* context compilation
* token budgeting
* permission evaluation
* tool argument validation
* path sandboxing
* session serialization
* model response parsing

Integration tests required for:

* complete agent loop
* tool call → execution → result
* streaming
* cancellation
* MCP tool execution
* OpenAI-compatible provider
* TUI event handling

Use mocked model providers.

Tests must NOT require a real API key.

---

# 34. Acceptance Test

After implementation:

```bash
cargo build --release
```

must produce:

```text
anvil
```

Then:

```bash
./target/release/anvil
```

must open the TUI.

Given a test repository:

```text
hello/
├── ANVIL.md
├── src/
│   └── main.rs
└── Cargo.toml
```

User enters:

```text
Add a function that returns the Fibonacci number for n.
Then write tests.
```

Anvil must:

1. inspect the project
2. read relevant files
3. propose/perform edits
4. request approval according to permissions
5. run tests
6. display results
7. provide final summary

---

# 35. MVP Definition of Done

Anvil is MVP-complete when this works:

```bash
cd my-project
anvil
```

User:

```text
Find the bug causing the failing tests and fix it.
```

Anvil:

```text
→ understands repository
→ reads relevant files
→ searches code
→ calls model
→ invokes tools
→ asks permission when required
→ edits files
→ runs tests
→ observes failures
→ iterates
→ reports final result
```

The user should feel that they are interacting with an
**engineering agent**, not a chatbot.

---

# 36. Implementation Order

Implement in this order.

## Phase 1 — Skeleton

* Rust workspace
* CLI
* configuration
* TUI
* event loop

## Phase 2 — Model

* provider abstraction
* OpenAI-compatible provider
* streaming
* message types

## Phase 3 — Agent

* agent loop
* tool calls
* event system
* cancellation

## Phase 4 — Tools

* read_file
* list_files
* search
* write_file
* edit_file
* shell
* git_status
* git_diff

## Phase 5 — Context

* workspace discovery
* ANVIL.md
* git state
* relevant file selection
* token budgeting

## Phase 6 — Skills

* SKILL.md parser
* global skills
* project skills
* skill discovery
* lazy skill loading

## Phase 7 — MCP

* MCP client
* server lifecycle
* tool discovery
* tool execution
* configuration

## Phase 8 — Sessions

* persistence
* history
* resume
* slash commands

## Phase 9 — Hardening

* permissions
* security
* error handling
* tests
* performance
* packaging

Do NOT start with:

* vector database
* embeddings
* autonomous planning framework
* multi-agent system
* custom model
* cloud infrastructure
* web UI

Build the local agent loop first.

---

# 37. Design Principle

The most important architectural boundary is:

```text
                 ANVIL
                   │
        ┌──────────┴──────────┐
        │                     │
   AGENT RUNTIME          MODEL PROVIDER
        │                     │
 context / skills / tools     │
        │                     │
        └──────────┬──────────┘
                   │
             chat/completions
```

The model generates intelligence.

**Anvil provides the environment in which that intelligence can act.**

That distinction must remain intact throughout the implementation.

```

I'd build the first version **Rust + Ratatui + Tokio**, with the OpenAI-compatible endpoint as the only model contract. That gives us a clean foundation to later plug in your own inference stack/Muse Spark without rewriting the agent.

The most important thing for the coding LLM is **not to implement everything at once**: Phase 1–4 should produce a working `anvil` that can inspect, edit, run, and iterate on a repository before MCP/skills/context optimization gets added.