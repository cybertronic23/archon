# Archon

Rust agent harness framework. Workspace with 4 crates.

## Build & Run

```bash
cargo build
cargo run -p archon-cli -- --api-key $ANTHROPIC_API_KEY
# or
ANTHROPIC_API_KEY=sk-... cargo run -p archon-cli
```

CLI flags: `--model` (default `claude-sonnet-4-20250514`), `--max-tokens` (default 8192).

## Project Structure

```
crates/
  archon-core/    — Types, Tool trait, ToolRegistry, Session, agent loop (StreamProvider trait + run_agent_loop)
  archon-llm/     — AnthropicProvider, SSE stream parser, Provider trait
  archon-tools/   — ReadTool, BashTool, EditTool implementations
  archon-cli/     — Binary entry point, clap args, REPL loop
```

Dependency direction: `cli → {core, llm, tools}`, `llm → core`, `tools → core`. `core` has no internal deps.

## Key Conventions

- All tools implement `archon_core::Tool` async trait (name, description, input_schema, execute).
- Tools are registered in `ToolRegistry` (HashMap-based). Adding a tool = implement trait + register in main.rs.
- LLM providers implement `archon_core::StreamProvider` returning `BoxStream<StreamEvent>`.
- Session messages follow Anthropic API format: tool results go in User messages.
- Agent loop is in `archon-core/src/agent_loop.rs` — the while-tool_use loop.
- SSE parsing is manual line-buffered in `archon-llm/src/anthropic.rs`, with event→struct mapping in `streaming.rs`.
- Error handling: tool errors → `ToolResult { is_error: true }` (LLM can retry). Transport errors → abort turn.

## Architecture Reference

See `ARCHITECTURE.md` for detailed design doc with data flow diagrams.
