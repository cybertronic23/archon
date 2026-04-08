use archon_core::Tool;
use archon_tools::{BashTool, SandboxMode};
use serde_json::json;

/// Helper: run a command through BashTool with given sandbox mode.
async fn run_bash(mode: SandboxMode, command: &str) -> String {
    let tool = BashTool::with_sandbox(mode);
    tool.execute(json!({ "command": command }))
        .await
        .unwrap_or_else(|e| format!("ERROR: {e}"))
}

#[tokio::test]
async fn test_off_mode_basic() {
    let output = run_bash(SandboxMode::Off, "echo hello-archon").await;
    assert!(output.contains("hello-archon"), "got: {output}");
}

#[tokio::test]
async fn test_permissive_basic_command() {
    // Should execute normally inside container
    let output = run_bash(SandboxMode::Permissive, "echo sandbox-permissive").await;
    assert!(
        output.contains("sandbox-permissive"),
        "expected output from container, got: {output}"
    );
}

#[tokio::test]
async fn test_permissive_network_blocked() {
    // Network should be disabled — curl/ping should fail
    let output = run_bash(
        SandboxMode::Permissive,
        // Use a simple network test; timeout quickly
        "bash -c 'echo test | /dev/tcp/8.8.8.8/53' 2>&1 || echo NETWORK_BLOCKED",
    )
    .await;
    assert!(
        output.contains("NETWORK_BLOCKED") || output.contains("Network is unreachable") || output.contains("Connection refused"),
        "network should be blocked in permissive mode, got: {output}"
    );
}

#[tokio::test]
async fn test_permissive_can_write_workspace() {
    // Should be able to write files in /workspace (cwd mount)
    let output = run_bash(
        SandboxMode::Permissive,
        "touch /workspace/.archon-sandbox-test && echo WRITE_OK && rm /workspace/.archon-sandbox-test",
    )
    .await;
    assert!(
        output.contains("WRITE_OK"),
        "should be able to write to workspace in permissive mode, got: {output}"
    );
}

#[tokio::test]
async fn test_strict_basic_command() {
    let output = run_bash(SandboxMode::Strict, "echo sandbox-strict").await;
    assert!(
        output.contains("sandbox-strict"),
        "expected output from strict container, got: {output}"
    );
}

#[tokio::test]
async fn test_strict_readonly_workspace() {
    // Workspace is mounted read-only in strict mode
    let output = run_bash(
        SandboxMode::Strict,
        "touch /workspace/.archon-test-strict 2>&1 || echo READONLY_OK",
    )
    .await;
    assert!(
        output.contains("READONLY_OK") || output.contains("Read-only file system"),
        "workspace should be read-only in strict mode, got: {output}"
    );
}

#[tokio::test]
async fn test_strict_network_blocked() {
    let output = run_bash(
        SandboxMode::Strict,
        "bash -c 'echo test > /dev/tcp/8.8.8.8/53' 2>&1 || echo NETWORK_BLOCKED",
    )
    .await;
    assert!(
        output.contains("NETWORK_BLOCKED") || output.contains("Network is unreachable"),
        "network should be blocked in strict mode, got: {output}"
    );
}

#[tokio::test]
async fn test_off_mode_timeout() {
    let tool = BashTool::with_sandbox(SandboxMode::Off);
    let result = tool
        .execute(json!({ "command": "sleep 10", "timeout": 1 }))
        .await;
    assert!(result.is_err(), "should timeout");
    let err = result.unwrap_err().to_string();
    assert!(err.contains("timed out"), "got: {err}");
}
