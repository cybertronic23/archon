# Archon — Architecture Design Document

## Overview

Archon (Greek: "ruler") is a Rust-based Agent Harness framework modeled after Claude Code. It implements a minimal, fully functional **agent loop**: user input → LLM streaming call → tool_use execution → result feedback → loop. Phase 1 ships with 3 core tools (Read, Bash, Edit) and a terminal REPL interface.

## Design Principles

- **Trait-driven extensibility** — New tools and LLM providers plug in via async traits, zero changes to core logic.
- **Streaming-first** — LLM output is printed token-by-token; SSE events are parsed incrementally.
- **Minimal surface area** — Each crate owns exactly one concern. No framework-level magic.
- **Explicit over implicit** — Tool results are plain strings. Session history is a flat `Vec<Message>`. No hidden state machines.

## Crate Dependency Graph

```
archon-cli
  ├── archon-core     (types, traits, agent loop)
  ├── archon-llm      (Anthropic streaming client)
  │     └── archon-core
  └── archon-tools    (Read, Bash, Edit)
        └── archon-core
```

`archon-core` is the foundation with zero internal crate dependencies. All other crates depend on it for shared types and traits.

## Crate Responsibilities

### archon-core

The kernel of the system. Defines all shared abstractions and orchestrates the agent loop.

| Module | Purpose |
|--------|---------|
| `types.rs` | `Message`, `ContentBlock`, `StopReason`, `StreamEvent`, `ToolDefinition`, `Usage` |
| `tool.rs` | `Tool` async trait + `ToolRegistry` (HashMap-based dispatch) |
| `session.rs` | `Session` — conversation history holder with typed push methods |
| `agent_loop.rs` | `StreamProvider` trait + `run_agent_loop()` orchestration function |

**Key types:**

```
Message { role: Role, content: Vec<ContentBlock> }

ContentBlock
  ├── Text { text }
  ├── ToolUse { id, name, input }
  └── ToolResult { tool_use_id, content, is_error }

StreamEvent
  ├── MessageStart { id, usage }
  ├── ContentBlockStart { index, content_block }
  ├── ContentBlockDelta { index, delta }
  ├── ContentBlockStop { index }
  ├── MessageDelta { stop_reason, usage }
  ├── MessageStop
  ├── Ping
  └── Error { message }
```

### archon-llm

Handles HTTP communication with the Anthropic Messages API and SSE stream parsing.

| Module | Purpose |
|--------|---------|
| `provider.rs` | `Provider` trait — abstraction for future multi-provider support |
| `streaming.rs` | `parse_stream_event()` — maps SSE event type + JSON data to `StreamEvent` |
| `anthropic.rs` | `AnthropicProvider` — POST with `stream: true`, SSE byte-stream parsing via mpsc channel |

**API contract:**
- Endpoint: `POST https://api.anthropic.com/v1/messages`
- Headers: `x-api-key`, `anthropic-version: 2023-06-01`, `content-type: application/json`
- Body: `{ model, max_tokens, stream: true, system, messages, tools }`
- Response: SSE byte stream → parsed into `BoxStream<'static, Result<StreamEvent>>`

**SSE parsing pipeline:**

```
HTTP response bytes
  → tokio::spawn line-buffered parser
    → accumulate "event:" and "data:" lines
    → on empty line: parse_stream_event(type, data)
  → mpsc::channel(64)
    → BoxStream<StreamEvent>
```

The mpsc channel (capacity 64) provides backpressure between the network reader and the agent loop consumer.

### archon-tools

Concrete tool implementations, each a unit struct implementing the `Tool` trait.

| Tool | Struct | Input | Behavior |
|------|--------|-------|----------|
| `read` | `ReadTool` | `file_path`, `offset?`, `limit?` | Read file, return cat -n formatted output |
| `bash` | `BashTool` | `command`, `timeout?` | Execute via `bash -c`, capture stdout/stderr, default 120s timeout |
| `edit` | `EditTool` | `file_path`, `old_string`, `new_string`, `replace_all?` | Exact string replacement; validates uniqueness before writing |

**Safety invariants:**
- `EditTool` refuses replacement if `old_string` appears 0 times (not found) or >1 time (ambiguous), unless `replace_all` is set.
- `BashTool` enforces a configurable timeout via `tokio::time::timeout`.
- `ReadTool` clamps offset/limit to file bounds; never panics on out-of-range.

### archon-cli

Binary entry point. Minimal glue code: parse args → wire components → run REPL.

- CLI args via `clap` derive: `--api-key` (or `$ANTHROPIC_API_KEY`), `--model`, `--max-tokens`
- Registers all three tools into `ToolRegistry`
- Uses `rustyline` for REPL with line editing, history, and multi-line input
- Runs an async REPL loop, calling `run_agent_loop()` per turn
- Auto-saves session to `~/.archon/sessions/latest.json` after each turn
- Handles EOF (Ctrl+D) for graceful exit

## Agent Loop — Detailed Flow

```
                    ┌─────────────────┐
                    │  User Input     │
                    │  (REPL stdin)   │
                    └────────┬────────┘
                             │
                             ▼
                    ┌─────────────────┐
                    │  session.push   │
                    │  _user(text)    │
                    └────────┬────────┘
                             │
              ┌──────────────┴──────────────┐
              │      run_agent_loop()       │
              │                             │
              │  ┌───────────────────────┐  │
         ┌───►│  │ provider.stream_msg() │  │
         │    │  └───────────┬───────────┘  │
         │    │              │              │
         │    │              ▼              │
         │    │  ┌───────────────────────┐  │
         │    │  │ Process StreamEvents  │  │
         │    │  │                       │  │
         │    │  │ • TextDelta → stdout  │  │
         │    │  │ • InputJsonDelta →    │  │
         │    │  │   accumulate buffer   │  │
         │    │  │ • BlockStop →         │  │
         │    │  │   finalize block      │  │
         │    │  │ • MessageDelta →      │  │
         │    │  │   capture stop_reason │  │
         │    │  └───────────┬───────────┘  │
         │    │              │              │
         │    │              ▼              │
         │    │  ┌───────────────────────┐  │
         │    │  │ push_assistant(blocks)│  │
         │    │  └───────────┬───────────┘  │
         │    │              │              │
         │    │              ▼              │
         │    │     stop_reason check      │
         │    │      ┌───────┴───────┐     │
         │    │      │               │     │
         │    │   ToolUse        EndTurn   │
         │    │      │           (break)   │
         │    │      ▼                     │
         │    │  ┌──────────────────┐      │
         │    │  │ Execute tools    │      │
         │    │  │ via ToolRegistry │      │
         │    │  └────────┬─────────┘      │
         │    │           │                │
         │    │           ▼                │
         │    │  ┌──────────────────┐      │
         │    │  │ push_tool_results│      │
         │    │  └────────┬─────────┘      │
         │    │           │                │
         └────┼───────────┘                │
              └────────────────────────────┘
```

**Per-block accumulation strategy:**

The agent loop maintains parallel `Vec`s indexed by content block position:
- `block_texts[i]` — accumulated text for text blocks
- `block_json_bufs[i]` — accumulated partial JSON for tool_use blocks
- `block_tool_ids[i]` / `block_tool_names[i]` — tool metadata
- `block_types[i]` — discriminant (`Text` | `ToolUse`)

On `ContentBlockStop`, the accumulated data is finalized into a `ContentBlock` and pushed to the response vector.

## Session Message Protocol

The session maintains a strict message sequence following the Anthropic API contract:

```
[System Prompt]  (passed separately, not in messages array)

User:      [Text("user question")]
Assistant: [Text("..."), ToolUse { id: "tu_1", name: "read", input: {...} }]
User:      [ToolResult { tool_use_id: "tu_1", content: "file contents..." }]
Assistant: [Text("Based on the file...")]
User:      [Text("next question")]
...
```

Key rule: **Tool results are always wrapped in User messages**, per API requirements.

## Error Handling Strategy

| Layer | Mechanism | Behavior |
|-------|-----------|----------|
| Tool execution | `Result<String>` | Errors become `ToolResult { is_error: true }` — LLM sees the error and can retry |
| SSE parsing | `Result<StreamEvent>` | Parse failures propagate up and abort the current turn |
| HTTP errors | Status code check | Non-2xx responses bail with status + body text |
| Agent loop | `Result<()>` | Errors printed to stderr in REPL; session preserved for retry |

This means tool errors are **recoverable** (the LLM can adapt), while transport/parsing errors are **fatal** to the current turn but not to the session.

## Extension Points

### Adding a new tool

1. Create a struct in `archon-tools/src/` implementing `archon_core::Tool`
2. Register it in `main.rs`: `tools.register(Box::new(MyTool))`

No changes needed to core, llm, or the agent loop.

### Adding a new LLM provider

1. Create a struct implementing `archon_core::StreamProvider`
2. Pass it to `run_agent_loop()` instead of `AnthropicProvider`

The agent loop is provider-agnostic — it only consumes `StreamEvent`s.

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1 (full) | Async runtime, process spawning, timers |
| `serde` / `serde_json` | 1 | Serialization for API payloads |
| `reqwest` | 0.12 (stream, json) | HTTP client with streaming response support |
| `futures` | 0.3 | `Stream` trait, `BoxStream`, `StreamExt` |
| `async-trait` | 0.1 | Async methods in traits |
| `anyhow` / `thiserror` | 1 / 2 | Error handling |
| `clap` | 4 (derive, env) | CLI argument parsing |
| `rustyline` | 15 (with-file-history) | REPL line editing, history, multi-line input |
| `dirs` | 6 | Platform-native home directory resolution |
| `eventsource-stream` | 0.2 | SSE stream utilities (declared but parsing done manually) |

## Permission System

Phase 2 adds a permission gate before every non-Safe tool execution, preventing the LLM from running arbitrary commands without user consent.

### Risk Classification

Every tool call is classified into one of three risk levels:

| Level | Default Tools | Behavior |
|-------|--------------|----------|
| `Safe` | `read` | Execute immediately, no prompt |
| `Moderate` | `edit`, unknown tools | Require user confirmation |
| `Dangerous` | `bash` | Require user confirmation |

Classification is handled by `PermissionHandler::classify()`, which can be overridden per-implementation.

### Core Abstractions (`archon-core/src/permission.rs`)

```
PermissionHandler (async trait)
  ├── classify(tool_name, input) → RiskLevel       // risk assessment
  └── check(PermissionRequest) → PermissionVerdict  // allow or deny

PermissionRequest { tool_name, input, risk_level }
PermissionVerdict { Allow | Deny }
```

Two built-in implementations:

| Struct | Module | Behavior |
|--------|--------|----------|
| `AllowAllPermissions` | `archon-core` | Always returns `Allow` — backward-compatible default |
| `InteractivePermissionHandler` | `archon-cli` | Prompts via stdin: `y`=allow, `n`=deny, `a`=always-allow this tool |

### Agent Loop Integration

In `run_agent_loop()`, before each tool execution:

```
for each ToolUse block:
    risk = permissions.classify(name, input)
    if risk != Safe:
        verdict = permissions.check(request)
        if verdict == Deny:
            → push ToolResult { is_error: true, "Permission denied" }
            → continue (skip execution, LLM sees the denial)
    → execute tool normally
```

Denied tools still produce a `ToolResult` so the LLM can observe the denial and adjust its behavior.

### CLI Flags

| Flag | Effect |
|------|--------|
| `--allow-all` | Use `AllowAllPermissions` (skip all prompts) |
| _(default)_ | Use `InteractivePermissionHandler` (prompt for Moderate/Dangerous) |

### Interactive Prompt UX

```
[Permission required] tool=bash risk=Dangerous
{
  "command": "rm -rf /tmp/test"
}
Allow? [y]es / [n]o / [a]lways: _
```

The `always` option adds the tool name to an in-memory set — subsequent calls to the same tool skip prompting for the rest of the session.

`spawn_blocking` is used for stdin reads to avoid blocking the tokio runtime.

## Sandbox (Docker-based)

Phase 2 adds optional sandboxed execution for bash commands using Docker containers via the `bollard` crate.

### Sandbox Modes

| Mode | Network | Filesystem | Resource Limits | Use Case |
|------|---------|-----------|----------------|----------|
| `Off` | Full | Full | None | Development, trusted environment |
| `Permissive` | Blocked | cwd mounted read-write | None | Block network exfiltration, allow file edits |
| `Strict` | Blocked | cwd mounted read-only | 512MB RAM, 50% CPU, 256 PIDs, drop ALL caps | Untrusted code, maximum isolation |

### Architecture

```
BashTool
  ├── sandbox_mode == Off  → tokio::process::Command("bash -c ...")
  └── sandbox_mode != Off  → DockerSandbox::execute()
                               ├── Create container (ubuntu:latest)
                               │   ├── Mount cwd → /workspace
                               │   ├── network_disabled: true
                               │   └── HostConfig (per mode)
                               ├── Start container
                               ├── Wait with timeout
                               ├── Collect logs (stdout + stderr)
                               └── Remove container (force)
```

### Docker Sandbox Lifecycle

```
DockerSandbox::new(mode, working_dir)
  → bollard::Docker::connect_with_local_defaults()
  → ping() to verify daemon is reachable
  → store Arc<Docker> client

DockerSandbox::execute(command, timeout)
  → create_container(Config { image, cmd, host_config, network_disabled })
  → start_container()
  → wait_container() with tokio::time::timeout
  → logs() to collect stdout/stderr
  → remove_container(force: true)
  → on timeout: kill_container() + remove
  → on error: cleanup + propagate
```

The Docker client is lazily initialized via `tokio::sync::OnceCell` — no Docker connection is attempted when `--sandbox off` (the default).

### Strict Mode Resource Limits

| Resource | Limit | Rationale |
|----------|-------|-----------|
| Memory | 512 MB | Prevent OOM from runaway processes |
| CPU | 50% (quota 50000 / period 100000) | Prevent host CPU starvation |
| PIDs | 256 | Prevent fork bombs |
| Capabilities | Drop ALL, add DAC_OVERRIDE only | Minimal privilege principle |
| Root filesystem | Writable (container-internal only) | Commands need /tmp; cwd is read-only via bind mount |

### CLI Flags

| Flag | Values | Default | Effect |
|------|--------|---------|--------|
| `--sandbox` | `off`, `permissive`, `strict` | `off` | Set sandbox mode for bash tool |

### Error Handling

| Scenario | Behavior |
|----------|----------|
| Docker not installed/running | Clear error message with hint: "start Docker or use --sandbox off" |
| Container creation fails | Error propagated to tool result |
| Command timeout | Container killed + removed, timeout error returned |
| Image not found | Docker pull needed (user responsibility, logged in error) |

## Test Plan

### Sandbox Integration Tests (`archon-tools/tests/sandbox_test.rs`)

| Test | Mode | Assertion |
|------|------|-----------|
| `test_off_mode_basic` | Off | `echo` output captured correctly |
| `test_off_mode_timeout` | Off | 10s sleep with 1s timeout → timeout error |
| `test_permissive_basic_command` | Permissive | `echo` works inside container |
| `test_permissive_network_blocked` | Permissive | `/dev/tcp` connection fails |
| `test_permissive_can_write_workspace` | Permissive | `touch` in /workspace succeeds |
| `test_strict_basic_command` | Strict | `echo` works inside container |
| `test_strict_network_blocked` | Strict | `/dev/tcp` connection fails |
| `test_strict_readonly_workspace` | Strict | `touch` in /workspace fails (read-only) |

Run tests: `cargo test -p archon-tools --test sandbox_test`

Prerequisites: Docker daemon running, `ubuntu:latest` image pulled.

### Manual Verification

1. **Permission system (interactive)**:
   ```bash
   cargo run -p archon-cli
   # Ask LLM to run a bash command → [Permission required] prompt appears
   # Type 'n' → LLM receives "Permission denied" and adjusts
   # Type 'a' → subsequent bash calls skip prompting
   ```

2. **Permission system (allow-all)**:
   ```bash
   cargo run -p archon-cli -- --allow-all
   # All tools execute without prompting
   ```

3. **Sandbox permissive**:
   ```bash
   cargo run -p archon-cli -- --sandbox permissive
   # Ask LLM to run curl → network blocked
   # Ask LLM to create a file in cwd → succeeds
   ```

4. **Sandbox strict**:
   ```bash
   cargo run -p archon-cli -- --sandbox strict
   # Ask LLM to write files → read-only filesystem error
   # Ask LLM to run curl → network blocked
   ```

## REPL Enhancement

The CLI uses `rustyline` (v15) for an enhanced terminal experience:

### Line Editing & History

- Full readline-compatible line editing (cursor movement, backspace, delete, etc.)
- Persistent command history saved to `~/.archon/history`
- Up/Down arrow keys browse previous inputs across sessions

### Multi-line Input

Lines ending with `\` trigger continuation mode:

```
You> Write a function that\
...> takes two arguments and\
...> returns their sum
```

The trailing `\` is stripped and lines are joined with `\n`. Ctrl+C cancels multi-line input.

### Directory Structure

```
~/.archon/
  history          # rustyline command history (persists across sessions)
  sessions/
    latest.json    # most recent session auto-save
```

## Session Persistence

Session state (system prompt, messages, token counts) is serialized to JSON for save/restore.

### Serialization

`Session` derives `Serialize`/`Deserialize` (all inner types already had these derives). Two methods:

- `save_to_file(&self, path: &Path)` — pretty-printed JSON, creates parent directories
- `load_from_file(path: &Path)` — deserialize from JSON file

### Auto-save

After each completed agent loop turn, the session is saved to `{session_dir}/latest.json`. On exit, the final state is also saved.

### Resume Flow

```
cargo run -p archon-cli -- --resume
```

1. Check if `{session_dir}/latest.json` exists
2. Deserialize into `Session`
3. Print summary (message count, token usage)
4. Continue REPL with restored history

If loading fails, a warning is printed and a fresh session is created.

### CLI Flags

| Flag | Default | Effect |
|------|---------|--------|
| `--session-dir <PATH>` | `~/.archon/sessions` | Override session storage directory |
| `--resume` | `false` | Load `latest.json` and resume conversation |

## Future Phases

- **Context window management** — message compression / summarization on overflow
- **Tool parallelism** — concurrent execution of independent tool_use blocks
- **Rich terminal UI** — syntax highlighting, spinners, markdown rendering
- **Custom tool loading** — dynamic tool registration from config or plugins
