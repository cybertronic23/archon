mod permission;

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;
use archon_core::{
    run_agent_loop, AllowAllPermissions, ContextConfig, PermissionHandler, Session, StreamProvider,
    ToolRegistry,
};
use archon_llm::{AnthropicProvider, OpenAIProvider, RetryConfig};
use archon_tools::{BashTool, EditTool, GlobTool, GrepTool, ReadTool, SandboxMode, WriteTool};
use clap::{CommandFactory, Parser};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde::Deserialize;

use crate::permission::InteractivePermissionHandler;

#[derive(Parser, Debug)]
#[command(name = "archon", about = "Archon — a Rust agent harness for Claude")]
struct Args {
    /// LLM provider: "anthropic" or "openai"
    #[arg(long, default_value = "openai")]
    provider: String,

    /// API key (also reads ANTHROPIC_API_KEY, OPENAI_API_KEY, or DASHSCOPE_API_KEY env vars)
    #[arg(long)]
    api_key: Option<String>,

    /// Custom API base URL (overrides provider default)
    #[arg(long)]
    base_url: Option<String>,

    /// Model to use (defaults: anthropic=claude-sonnet-4-20250514, openai=qwen-plus)
    #[arg(long)]
    model: Option<String>,

    /// Max tokens per response
    #[arg(long, default_value_t = 8192)]
    max_tokens: u32,

    /// Allow all tool executions without prompting
    #[arg(long, default_value_t = false)]
    allow_all: bool,

    /// Sandbox mode for bash commands: off, permissive, strict
    #[arg(long, default_value = "off")]
    sandbox: SandboxMode,

    /// Max retries for transient API errors (429, 5xx)
    #[arg(long, default_value_t = 3)]
    max_retries: u32,

    /// Max context window tokens (default: 200000 for anthropic, 128000 for openai)
    #[arg(long)]
    max_context_tokens: Option<u64>,

    /// Session directory for persistence (default: ~/.archon/sessions)
    #[arg(long)]
    session_dir: Option<PathBuf>,

    /// Resume the latest saved session
    #[arg(long, default_value_t = false)]
    resume: bool,

    /// Custom system prompt string (overrides default)
    #[arg(long)]
    system_prompt: Option<String>,

    /// Load system prompt from a file path (overrides default)
    #[arg(long, conflicts_with = "system_prompt")]
    system_prompt_file: Option<PathBuf>,
}

const SYSTEM_PROMPT: &str = r#"You are Archon, a helpful AI assistant that can read files, execute bash commands, and edit files.

When asked to perform tasks, use the available tools:
- **read**: Read file contents. Provide file_path, and optionally offset/limit.
- **bash**: Execute shell commands. Provide the command string.
- **edit**: Make exact string replacements in files. Provide file_path, old_string, and new_string. The old_string must be unique in the file.
- **write**: Create or overwrite a file with the given content. Parent directories are created automatically.
- **glob**: Find files matching a glob pattern (e.g. "**/*.rs"). Optionally specify a base path.
- **grep**: Search file contents using regex. Recursively searches directories, skipping binary files and hidden dirs.

Always use tools when appropriate rather than guessing at file contents or command outputs. You can chain multiple tool calls to accomplish complex tasks."#;

/// Configuration loaded from ~/.archon/config.toml.
/// All fields are optional; missing fields are left as None.
#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    provider: Option<String>,
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    max_tokens: Option<u32>,
    max_retries: Option<u32>,
    max_context_tokens: Option<u64>,
    sandbox: Option<String>,
    session_dir: Option<String>,
}

/// Load configuration from ~/.archon/config.toml.
/// Returns default (empty) config if the file does not exist.
fn load_config() -> Result<FileConfig> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let config_path = home.join(".archon").join("config.toml");
    if !config_path.exists() {
        return Ok(FileConfig::default());
    }
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| anyhow::anyhow!("Failed to read {}: {e}", config_path.display()))?;
    let config: FileConfig = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", config_path.display()))?;
    Ok(config)
}

/// Apply config file values to Args for fields not explicitly set via CLI.
/// Priority: CLI > config file > default.
fn apply_config(args: &mut Args, config: &FileConfig) {
    let matches = Args::command().get_matches_from(std::env::args_os());

    // Helper: returns true if the arg was explicitly passed on the command line
    let is_explicit = |id: &str| -> bool {
        matches
            .value_source(id)
            .map(|s| s == clap::parser::ValueSource::CommandLine)
            .unwrap_or(false)
    };

    if !is_explicit("provider") {
        if let Some(ref v) = config.provider {
            args.provider = v.clone();
        }
    }
    if !is_explicit("base_url") {
        // base_url is handled in build_provider via config fallback
    }
    if !is_explicit("model") {
        if let Some(ref v) = config.model {
            args.model = Some(v.clone());
        }
    }
    if !is_explicit("max_tokens") {
        if let Some(v) = config.max_tokens {
            args.max_tokens = v;
        }
    }
    if !is_explicit("max_retries") {
        if let Some(v) = config.max_retries {
            args.max_retries = v;
        }
    }
    if !is_explicit("max_context_tokens") {
        if let Some(v) = config.max_context_tokens {
            args.max_context_tokens = Some(v);
        }
    }
    if !is_explicit("sandbox") {
        if let Some(ref v) = config.sandbox {
            if let Ok(mode) = v.parse::<SandboxMode>() {
                args.sandbox = mode;
            }
        }
    }
    if !is_explicit("session_dir") {
        if let Some(ref v) = config.session_dir {
            let path = if v.starts_with('~') {
                if let Some(home) = dirs::home_dir() {
                    home.join(v.strip_prefix("~/").unwrap_or(v))
                } else {
                    PathBuf::from(v)
                }
            } else {
                PathBuf::from(v)
            };
            args.session_dir = Some(path);
        }
    }
}

/// Resolve system prompt from CLI args or use the default.
fn resolve_system_prompt(args: &Args) -> Result<String> {
    if let Some(ref prompt) = args.system_prompt {
        return Ok(prompt.clone());
    }
    if let Some(ref path) = args.system_prompt_file {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read system prompt file '{}': {e}", path.display()))?;
        return Ok(content);
    }
    Ok(SYSTEM_PROMPT.to_string())
}

/// Return the ~/.archon directory, creating it if needed.
fn archon_home() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Cannot determine home directory"))?;
    let dir = home.join(".archon");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Resolve API key: CLI flag > environment variables > config file.
fn resolve_api_key(explicit: Option<String>, provider: &str, config_key: Option<&str>) -> Result<String> {
    if let Some(key) = explicit {
        return Ok(key);
    }

    let env_vars: &[&str] = match provider {
        "anthropic" => &["ANTHROPIC_API_KEY"],
        "openai" => &["OPENAI_API_KEY", "DASHSCOPE_API_KEY"],
        _ => &["OPENAI_API_KEY", "DASHSCOPE_API_KEY", "ANTHROPIC_API_KEY"],
    };

    for var in env_vars {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Ok(val);
            }
        }
    }

    if let Some(key) = config_key {
        if !key.is_empty() {
            return Ok(key.to_string());
        }
    }

    let var_list = env_vars.join(" or ");
    anyhow::bail!(
        "No API key found. Pass --api-key, set {var_list}, or add api_key to ~/.archon/config.toml."
    );
}

/// Build the appropriate provider based on CLI args and config.
fn build_provider(args: &Args, config: &FileConfig) -> Result<Box<dyn StreamProvider>> {
    let api_key = resolve_api_key(args.api_key.clone(), &args.provider, config.api_key.as_deref())?;
    let retry_config = RetryConfig {
        max_retries: args.max_retries,
        ..RetryConfig::default()
    };

    let base_url = args.base_url.clone().or_else(|| config.base_url.clone());

    match args.provider.as_str() {
        "anthropic" => Ok(Box::new(
            AnthropicProvider::new(api_key).with_retry_config(retry_config),
        )),
        "openai" => {
            let provider = if let Some(ref url) = base_url {
                OpenAIProvider::with_base_url(api_key, url.clone())
            } else {
                OpenAIProvider::new(api_key)
            };
            Ok(Box::new(provider.with_retry_config(retry_config)))
        }
        other => anyhow::bail!("Unknown provider: {other}. Use 'anthropic' or 'openai'."),
    }
}

/// Read a potentially multi-line input from rustyline.
/// Lines ending with `\` are continuation lines; the prompt changes to `...> `.
fn read_multiline_input(rl: &mut DefaultEditor) -> Result<Option<String>, ReadlineError> {
    let mut lines = Vec::new();
    let mut prompt = "You> ";

    loop {
        match rl.readline(prompt) {
            Ok(line) => {
                if line.ends_with('\\') {
                    // Continuation: strip trailing backslash, switch to continuation prompt
                    lines.push(line[..line.len() - 1].to_string());
                    prompt = "...> ";
                } else {
                    lines.push(line);
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl+C: discard current multiline input, start fresh
                if !lines.is_empty() {
                    lines.clear();
                    println!("[Input cancelled]");
                    return Ok(None);
                }
                return Ok(None);
            }
            Err(e) => return Err(e),
        }
    }

    let input = lines.join("\n");
    if input.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(input))
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = Args::parse();

    // Load config file and apply defaults for fields not set via CLI
    let config = load_config()?;
    apply_config(&mut args, &config);

    let archon_home = archon_home()?;
    let history_path = archon_home.join("history");
    let session_dir = args
        .session_dir
        .clone()
        .unwrap_or_else(|| archon_home.join("sessions"));
    std::fs::create_dir_all(&session_dir)?;
    let session_file = session_dir.join("latest.json");

    let model = args.model.clone().unwrap_or_else(|| match args.provider.as_str() {
        "anthropic" => "claude-sonnet-4-20250514".to_string(),
        _ => "qwen-plus".to_string(),
    });

    let provider = build_provider(&args, &config)?;

    let context_config = ContextConfig {
        max_context_tokens: args
            .max_context_tokens
            .unwrap_or(match args.provider.as_str() {
                "anthropic" => 200_000,
                _ => 128_000,
            }),
        ..ContextConfig::default()
    };

    let mut tools = ToolRegistry::new();
    tools.register(Box::new(ReadTool));
    tools.register(Box::new(BashTool::with_sandbox(args.sandbox)));
    tools.register(Box::new(EditTool));
    tools.register(Box::new(WriteTool));
    tools.register(Box::new(GlobTool));
    tools.register(Box::new(GrepTool));

    let permissions: Box<dyn PermissionHandler> = if args.allow_all {
        Box::new(AllowAllPermissions)
    } else {
        Box::new(InteractivePermissionHandler::new())
    };

    // Load or create session
    let mut session = if args.resume && session_file.exists() {
        match Session::load_from_file(&session_file) {
            Ok(mut s) => {
                // Override system prompt if user explicitly provided one
                if args.system_prompt.is_some() || args.system_prompt_file.is_some() {
                    s.system_prompt = resolve_system_prompt(&args)?;
                }
                println!(
                    "[Resumed session: {} messages, {} input tokens, {} output tokens]",
                    s.messages.len(),
                    s.total_input_tokens,
                    s.total_output_tokens
                );
                s
            }
            Err(e) => {
                eprintln!("[Warning: failed to load session: {e}. Starting fresh.]");
                Session::new(resolve_system_prompt(&args)?)
            }
        }
    } else {
        Session::new(resolve_system_prompt(&args)?)
    };

    // Initialize rustyline editor
    let mut rl = DefaultEditor::new()?;
    let _ = rl.load_history(&history_path);

    println!(
        "Archon v0.1.0 (provider: {}, model: {}, sandbox: {}) — type your message (Ctrl+D to exit)",
        args.provider, model, args.sandbox
    );
    println!("  Tip: end a line with \\ for multi-line input. Use Up/Down for history.\n");

    loop {
        let input = match read_multiline_input(&mut rl) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(ReadlineError::Eof) => {
                println!("\nGoodbye!");
                break;
            }
            Err(e) => {
                eprintln!("[Readline error: {e}]");
                break;
            }
        };

        // Add to history
        let _ = rl.add_history_entry(&input);

        session.push_user(&input);

        print!("\nArchon> ");
        io::stdout().flush()?;

        if let Err(e) = run_agent_loop(
            provider.as_ref(),
            &tools,
            permissions.as_ref(),
            &mut session,
            &model,
            args.max_tokens,
            &context_config,
        )
        .await
        {
            eprintln!("\n[Error: {e}]");
        }

        println!();

        // Auto-save session after each turn
        if let Err(e) = session.save_to_file(&session_file) {
            eprintln!("[Warning: failed to save session: {e}]");
        }
    }

    // Save history and final session state on exit
    let _ = rl.save_history(&history_path);
    if let Err(e) = session.save_to_file(&session_file) {
        eprintln!("[Warning: failed to save session: {e}]");
    }

    Ok(())
}
