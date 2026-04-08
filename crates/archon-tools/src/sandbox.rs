use std::sync::Arc;

use anyhow::Result;
use bollard::container::{
    Config, CreateContainerOptions, LogsOptions, RemoveContainerOptions, WaitContainerOptions,
};
use bollard::models::HostConfig;
use futures::StreamExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxMode {
    Off,
    Permissive,
    Strict,
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxMode::Off => write!(f, "off"),
            SandboxMode::Permissive => write!(f, "permissive"),
            SandboxMode::Strict => write!(f, "strict"),
        }
    }
}

impl std::str::FromStr for SandboxMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "off" => Ok(SandboxMode::Off),
            "permissive" => Ok(SandboxMode::Permissive),
            "strict" => Ok(SandboxMode::Strict),
            other => Err(format!(
                "unknown sandbox mode: {other} (expected off|permissive|strict)"
            )),
        }
    }
}

/// Default Docker image for sandbox containers.
const DEFAULT_IMAGE: &str = "ubuntu:latest";

/// Docker-based sandbox executor.
pub struct DockerSandbox {
    client: Arc<bollard::Docker>,
    image: String,
    working_dir: String,
    mode: SandboxMode,
}

impl DockerSandbox {
    /// Try to connect to Docker daemon and create a sandbox executor.
    pub async fn new(mode: SandboxMode, working_dir: String) -> Result<Self> {
        let client = bollard::Docker::connect_with_local_defaults()
            .map_err(|e| anyhow::anyhow!("Failed to connect to Docker: {e}"))?;

        // Verify Docker is reachable
        client
            .ping()
            .await
            .map_err(|e| anyhow::anyhow!("Docker daemon not reachable: {e}"))?;

        Ok(Self {
            client: Arc::new(client),
            image: DEFAULT_IMAGE.to_string(),
            working_dir,
            mode,
        })
    }

    /// Execute a command inside a Docker container and return combined output.
    pub async fn execute(&self, command: &str, timeout_secs: u64) -> Result<String> {
        let container_name = format!("archon-sandbox-{}", uuid_simple());

        let host_config = self.build_host_config();

        let config = Config {
            image: Some(self.image.clone()),
            cmd: Some(vec![
                "bash".to_string(),
                "-c".to_string(),
                command.to_string(),
            ]),
            working_dir: Some("/workspace".to_string()),
            host_config: Some(host_config),
            network_disabled: Some(self.mode == SandboxMode::Strict),
            ..Default::default()
        };

        // Create container
        let create_opts = CreateContainerOptions {
            name: container_name.as_str(),
            platform: None,
        };
        self.client
            .create_container(Some(create_opts), config)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to create container: {e}"))?;

        // Start container
        self.client
            .start_container::<String>(&container_name, None)
            .await
            .map_err(|e| {
                // Clean up on start failure
                let client = self.client.clone();
                let name = container_name.clone();
                tokio::spawn(async move { remove_container(&client, &name).await });
                anyhow::anyhow!("Failed to start container: {e}")
            })?;

        // Wait for container with timeout
        let wait_result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            wait_for_container(&self.client, &container_name),
        )
        .await;

        let exit_code = match wait_result {
            Ok(Ok(code)) => code,
            Ok(Err(e)) => {
                // Kill + remove on error
                self.client.kill_container::<String>(&container_name, None).await.ok();
                remove_container(&self.client, &container_name).await;
                return Err(e);
            }
            Err(_) => {
                // Timeout — kill container
                self.client.kill_container::<String>(&container_name, None).await.ok();
                remove_container(&self.client, &container_name).await;
                anyhow::bail!("Command timed out after {timeout_secs} seconds");
            }
        };

        // Collect logs
        let output = collect_logs(&self.client, &container_name).await?;

        // Remove container
        remove_container(&self.client, &container_name).await;

        // Format output
        let mut result = output;
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

    fn build_host_config(&self) -> HostConfig {
        let bind = format!("{}:/workspace", self.working_dir);

        match self.mode {
            SandboxMode::Permissive => HostConfig {
                binds: Some(vec![bind]),
                ..Default::default()
            },
            SandboxMode::Strict => HostConfig {
                binds: Some(vec![format!("{bind}:ro")]),
                memory: Some(512 * 1024 * 1024), // 512MB
                cpu_period: Some(100_000),
                cpu_quota: Some(50_000), // 50% CPU
                pids_limit: Some(256),
                cap_drop: Some(vec![
                    "ALL".to_string(),
                ]),
                cap_add: Some(vec![
                    "DAC_OVERRIDE".to_string(),
                ]),
                readonly_rootfs: Some(false),
                ..Default::default()
            },
            SandboxMode::Off => HostConfig {
                binds: Some(vec![bind]),
                ..Default::default()
            },
        }
    }
}

async fn wait_for_container(client: &bollard::Docker, name: &str) -> Result<i64> {
    let opts = WaitContainerOptions {
        condition: "not-running",
    };
    let mut stream = client.wait_container(name, Some(opts));
    if let Some(result) = stream.next().await {
        let resp = result.map_err(|e| anyhow::anyhow!("Error waiting for container: {e}"))?;
        Ok(resp.status_code)
    } else {
        Ok(0)
    }
}

async fn collect_logs(client: &bollard::Docker, name: &str) -> Result<String> {
    let opts = LogsOptions::<String> {
        stdout: true,
        stderr: true,
        follow: false,
        ..Default::default()
    };
    let mut stream = client.logs(name, Some(opts));
    let mut output = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(log) => output.push_str(&log.to_string()),
            Err(e) => {
                anyhow::bail!("Error reading container logs: {e}");
            }
        }
    }
    Ok(output)
}

async fn remove_container(client: &bollard::Docker, name: &str) {
    let opts = RemoveContainerOptions {
        force: true,
        ..Default::default()
    };
    client.remove_container(name, Some(opts)).await.ok();
}

/// Simple unique ID without pulling in uuid crate.
fn uuid_simple() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{ts:x}-{pid:x}")
}
