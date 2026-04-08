use anyhow::Result;
use archon_core::Tool;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::{Mutex, OnceCell};

use crate::sandbox::{DockerSandbox, SandboxMode};

/// Marker used to separate real command output from the pwd probe.
const CWD_MARKER: &str = "\n__ARCHON_CWD_PROBE__";

pub struct BashTool {
    pub sandbox_mode: SandboxMode,
    /// Tracked working directory, protected by a mutex for concurrent access.
    working_dir: Mutex<String>,
    /// Lazily initialized Docker sandbox (only when mode != Off).
    docker: OnceCell<Arc<DockerSandbox>>,
}

impl BashTool {
    pub fn new() -> Self {
        Self {
            sandbox_mode: SandboxMode::Off,
            working_dir: Mutex::new(current_dir_string()),
            docker: OnceCell::new(),
        }
    }

    pub fn with_sandbox(mode: SandboxMode) -> Self {
        Self {
            sandbox_mode: mode,
            working_dir: Mutex::new(current_dir_string()),
            docker: OnceCell::new(),
        }
    }

    /// Get or initialize the Docker sandbox client.
    async fn get_docker(&self) -> Result<&Arc<DockerSandbox>> {
        self.docker
            .get_or_try_init(|| async {
                let wd = self.working_dir.lock().await.clone();
                let sandbox = DockerSandbox::new(self.sandbox_mode, wd).await?;
                Ok(Arc::new(sandbox))
            })
            .await
    }
}

fn current_dir_string() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| ".".to_string())
}

/// Wrap a command to capture the final working directory after execution.
/// Appends a pwd probe that we strip from the output later.
fn wrap_command_with_cwd_probe(command: &str) -> String {
    // Run the original command, capture its exit code, print a marker + pwd, exit with original code.
    format!(
        "{command}\n__archon_ec__=$?\nprintf '{CWD_MARKER}\\n'\npwd\nexit $__archon_ec__"
    )
}

/// Parse the output to separate real output from the pwd probe.
/// Returns (real_output, new_cwd_option).
fn parse_cwd_probe(raw_output: &str) -> (String, Option<String>) {
    if let Some(marker_pos) = raw_output.rfind(CWD_MARKER) {
        let real_output = &raw_output[..marker_pos];
        let after_marker = &raw_output[marker_pos + CWD_MARKER.len()..];
        // The line after the marker is the pwd
        let new_cwd = after_marker
            .trim()
            .lines()
            .next()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        (real_output.to_string(), new_cwd)
    } else {
        (raw_output.to_string(), None)
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command and return its output (stdout and stderr combined). \
         Commands have a configurable timeout (default 120 seconds)."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 120)"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<String> {
        let command = input["command"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;

        let timeout_secs = input["timeout"].as_u64().unwrap_or(120);

        if self.sandbox_mode != SandboxMode::Off {
            // Docker sandbox execution — each container starts fresh, no cd tracking needed
            let docker = self.get_docker().await.map_err(|e| {
                anyhow::anyhow!(
                    "Docker sandbox unavailable (is Docker running?): {e}\n\
                     Hint: start Docker or use --sandbox off"
                )
            })?;
            return docker.execute(command, timeout_secs).await;
        }

        // Snapshot the current working directory (lock released immediately)
        let cwd = self.working_dir.lock().await.clone();

        // Wrap command with pwd probe to track directory changes
        let wrapped = wrap_command_with_cwd_probe(command);

        // Direct execution with tracked working directory
        let result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            Command::new("bash")
                .arg("-c")
                .arg(&wrapped)
                .current_dir(&cwd)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                // Parse the pwd probe from stdout
                let (clean_stdout, new_cwd) = parse_cwd_probe(&stdout);

                // Update tracked working directory if changed
                if let Some(new_dir) = new_cwd {
                    let mut wd = self.working_dir.lock().await;
                    *wd = new_dir;
                }

                let mut result = String::new();
                if !clean_stdout.is_empty() {
                    result.push_str(&clean_stdout);
                }
                if !stderr.is_empty() {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str("[stderr]\n");
                    result.push_str(&stderr);
                }
                if exit_code != 0 {
                    if !result.is_empty() {
                        result.push('\n');
                    }
                    result.push_str(&format!("[exit code: {exit_code}]"));
                }
                if result.is_empty() {
                    result = "(no output)".to_string();
                }
                Ok(result)
            }
            Ok(Err(e)) => Err(anyhow::anyhow!("Failed to execute command: {e}")),
            Err(_) => Err(anyhow::anyhow!(
                "Command timed out after {timeout_secs} seconds"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cwd_probe_with_output() {
        let raw = format!("hello world{CWD_MARKER}\n/home/user\n");
        let (output, cwd) = parse_cwd_probe(&raw);
        assert_eq!(output, "hello world");
        assert_eq!(cwd.unwrap(), "/home/user");
    }

    #[test]
    fn test_parse_cwd_probe_empty_output() {
        let raw = format!("{CWD_MARKER}\n/tmp\n");
        let (output, cwd) = parse_cwd_probe(&raw);
        assert_eq!(output, "");
        assert_eq!(cwd.unwrap(), "/tmp");
    }

    #[test]
    fn test_parse_cwd_probe_no_marker() {
        let raw = "just some output";
        let (output, cwd) = parse_cwd_probe(raw);
        assert_eq!(output, "just some output");
        assert!(cwd.is_none());
    }
}
