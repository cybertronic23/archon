//! `RobotBackend` that talks to an external sim worker over NDJSON stdio.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use archon_embodied::{CancelToken, JointCommand, Observation, RobotBackend};
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::protocol::{ClientMsg, ServerMsg, PROTOCOL_VERSION};

#[derive(Debug, Clone)]
pub struct BridgeConfig {
    pub platform: String,
    pub worker_cmd: Vec<String>,
    pub dof: usize,
    pub joint_names: Vec<String>,
    pub render: bool,
    pub media_root: Option<PathBuf>,
    /// Working directory for the worker process.
    pub cwd: Option<PathBuf>,
    pub model_path: Option<PathBuf>,
    pub camera: Option<String>,
    pub auto_joints: bool,
    /// Launch MuJoCo passive viewer (GUI).
    pub viewer: bool,
    pub record_dir: Option<PathBuf>,
    pub video_out: Option<PathBuf>,
    pub record_width: u32,
    pub record_height: u32,
}

impl BridgeConfig {
    pub fn mujoco(worker_script: impl Into<PathBuf>) -> Self {
        let script = worker_script.into();
        let script = std::fs::canonicalize(&script).unwrap_or(script);
        Self {
            platform: "mujoco".into(),
            worker_cmd: python_cmd(&script),
            dof: 6,
            joint_names: (1..=6).map(|i| format!("joint_{i}")).collect(),
            render: true,
            media_root: None,
            cwd: None,
            model_path: None,
            camera: None,
            auto_joints: true,
            viewer: false,
            record_dir: None,
            video_out: None,
            record_width: 1280,
            record_height: 720,
        }
    }

    pub fn maniskill(worker_script: impl Into<PathBuf>) -> Self {
        let mut c = Self::mujoco(worker_script);
        c.platform = "maniskill".into();
        c
    }

    pub fn with_media_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.media_root = Some(root.into());
        self
    }

    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn with_model(mut self, path: impl Into<PathBuf>) -> Self {
        self.model_path = Some(path.into());
        self
    }

    pub fn with_camera(mut self, camera: impl Into<String>) -> Self {
        self.camera = Some(camera.into());
        self
    }

    pub fn with_viewer(mut self, viewer: bool) -> Self {
        self.viewer = viewer;
        if viewer {
            // macOS: MuJoCo launch_passive must run under mjpython (Cocoa main thread).
            if let Some(cmd) = mjpython_worker_cmd(&self.worker_cmd) {
                eprintln!("[archon-sim-bridge] using mjpython for viewer: {}", cmd[0]);
                self.worker_cmd = cmd;
            } else if cfg!(target_os = "macos") {
                eprintln!(
                    "[archon-sim-bridge] warning: mjpython not found; viewer will fail on macOS. \
                     Ensure mujoco is installed in .venv-mujoco (provides bin/mjpython)."
                );
            }
        }
        self
    }

    pub fn with_record_video(mut self, out: impl Into<PathBuf>) -> Self {
        let out = out.into();
        let dir = out
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!(
                ".archon_frames_{}",
                out.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("demo")
            ));
        self.record_dir = Some(dir);
        self.video_out = Some(out);
        self.render = true;
        self
    }
}

fn python_cmd(script: &Path) -> Vec<String> {
    let py = resolve_python();
    vec![py, "-u".into(), script.to_string_lossy().into_owned()]
}

fn resolve_python() -> String {
    std::env::var("ARCHON_PYTHON")
        .ok()
        .filter(|p| Path::new(p).exists())
        .or_else(|| {
            // Prefer repo .venv-mujoco even when cwd is a subdirectory (e.g. tmp-episodes/).
            venv_mujoco_bin("python3")
                .or_else(|| venv_mujoco_bin("python"))
                .map(|p| p.to_string_lossy().into_owned())
        })
        .or_else(|| {
            ["python3", "python"]
                .into_iter()
                .find(|c| which_exists(c))
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "python3".into())
}

/// Locate `.venv-mujoco/bin/<name>` from cwd, parents, or crate workspace root.
fn venv_mujoco_bin(name: &str) -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        let mut d = cwd;
        for _ in 0..6 {
            roots.push(d.clone());
            if !d.pop() {
                break;
            }
        }
    }
    // crates/archon-sim-bridge → repo root
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));

    for root in roots {
        let p = root.join(".venv-mujoco/bin").join(name);
        // Do NOT canonicalize: following the venv symlink lands on the system
        // interpreter and drops site-packages (mujoco disappears).
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Rewrite worker_cmd to use mjpython (same venv / PATH) when available.
fn mjpython_worker_cmd(worker_cmd: &[String]) -> Option<Vec<String>> {
    if worker_cmd.len() < 3 {
        return None;
    }
    let script = worker_cmd[worker_cmd.len() - 1].clone();
    let current_py = Path::new(&worker_cmd[0]);

    let candidates = [
        std::env::var_os("ARCHON_MJPYTHON").map(PathBuf::from),
        current_py.parent().map(|d| d.join("mjpython")),
        venv_mujoco_bin("mjpython"),
        which_path("mjpython"),
    ];

    for c in candidates.into_iter().flatten() {
        if c.exists() {
            return Some(vec![
                c.to_string_lossy().into_owned(),
                "-u".into(),
                script,
            ]);
        }
    }
    None
}

fn which_path(cmd: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(cmd))
            .find(|p| p.is_file())
    })
}

fn which_exists(cmd: &str) -> bool {
    which_path(cmd).is_some()
}

struct BridgeIo {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct BridgedSimBackend {
    config: BridgeConfig,
    io: Arc<Mutex<Option<BridgeIo>>>,
    last_obs: Arc<Mutex<Option<Observation>>>,
}

impl BridgedSimBackend {
    pub fn new(config: BridgeConfig) -> Self {
        Self {
            config,
            io: Arc::new(Mutex::new(None)),
            last_obs: Arc::new(Mutex::new(None)),
        }
    }

    pub fn platform(&self) -> &str {
        &self.config.platform
    }

    async fn send_recv(&self, msg: ClientMsg) -> Result<ServerMsg> {
        let mut guard = self.io.lock().await;
        let io = guard
            .as_mut()
            .context("bridge not connected; call connect() first")?;

        let line = serde_json::to_string(&msg).context("serialize client msg")?;
        io.stdin
            .write_all(line.as_bytes())
            .await
            .context("write to worker stdin")?;
        io.stdin.write_all(b"\n").await?;
        io.stdin.flush().await?;

        let mut response = String::new();
        let n = io
            .stdout
            .read_line(&mut response)
            .await
            .context("read worker stdout")?;
        if n == 0 {
            bail!("worker closed stdout (EOF)");
        }
        let parsed: ServerMsg =
            serde_json::from_str(response.trim()).context("parse worker response")?;
        if let ServerMsg::Error { message } = &parsed {
            bail!("worker error: {message}");
        }
        Ok(parsed)
    }

    async fn request_observation(&self) -> Result<Observation> {
        let resp = self.send_recv(ClientMsg::Observe).await?;
        match resp {
            ServerMsg::Observation { .. } => {
                let obs = resp.into_observation()?;
                *self.last_obs.lock().await = Some(obs.clone());
                Ok(obs)
            }
            other => bail!("unexpected response to observe: {:?}", other),
        }
    }
}

#[async_trait]
impl RobotBackend for BridgedSimBackend {
    fn name(&self) -> &str {
        &self.config.platform
    }

    async fn connect(&mut self) -> Result<()> {
        if self.io.lock().await.is_some() {
            return Ok(());
        }
        let mut cmd = Command::new(&self.config.worker_cmd[0]);
        for arg in &self.config.worker_cmd[1..] {
            cmd.arg(arg);
        }
        if let Some(cwd) = &self.config.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);

        let mut child = cmd
            .spawn()
            .with_context(|| {
                format!(
                    "spawn worker {:?}. Install platform deps and check --worker path.",
                    self.config.worker_cmd
                )
            })?;

        let stdin = child.stdin.take().context("worker stdin")?;
        let stdout = child.stdout.take().context("worker stdout")?;
        let mut io = BridgeIo {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        };

        // Temporary assign for handshake via send_recv pattern — do inline:
        let hello = ClientMsg::Hello {
            protocol_version: PROTOCOL_VERSION,
            platform: self.config.platform.clone(),
            dof: self.config.dof,
            joint_names: self.config.joint_names.clone(),
            render: self.config.render,
            media_root: self
                .config
                .media_root
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            model_path: self
                .config
                .model_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            camera: self.config.camera.clone(),
            auto_joints: self.config.auto_joints,
            viewer: self.config.viewer,
            record_dir: self
                .config
                .record_dir
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            video_out: self
                .config
                .video_out
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            record_width: Some(self.config.record_width),
            record_height: Some(self.config.record_height),
        };
        let line = serde_json::to_string(&hello)?;
        io.stdin.write_all(line.as_bytes()).await?;
        io.stdin.write_all(b"\n").await?;
        io.stdin.flush().await?;

        let mut response = String::new();
        io.stdout.read_line(&mut response).await?;
        let parsed: ServerMsg = serde_json::from_str(response.trim())
            .with_context(|| format!("hello response: {response}"))?;
        match parsed {
            ServerMsg::HelloOk {
                platform,
                joint_names,
                dof,
                ..
            } => {
                if platform != self.config.platform {
                    bail!(
                        "platform mismatch: expected {}, worker said {}",
                        self.config.platform,
                        platform
                    );
                }
                if !joint_names.is_empty() {
                    self.config.joint_names = joint_names;
                    self.config.dof = if dof > 0 {
                        dof
                    } else {
                        self.config.joint_names.len()
                    };
                }
            }
            ServerMsg::Error { message } => bail!("worker hello failed: {message}"),
            other => bail!("expected hello_ok, got {:?}", other),
        }

        *self.io.lock().await = Some(io);

        // Reset scene and cache first observation.
        let resp = self.send_recv(ClientMsg::Reset).await?;
        let obs = match resp {
            ServerMsg::Observation { .. } => resp.into_observation()?,
            other => bail!("reset should return observation, got {:?}", other),
        };
        *self.last_obs.lock().await = Some(obs);

        eprintln!(
            "[archon-sim-bridge] connected platform={} worker={:?}",
            self.config.platform, self.config.worker_cmd
        );
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        if self.io.lock().await.is_some() {
            let _ = self.send_recv(ClientMsg::Shutdown).await;
            if let Some(mut io) = self.io.lock().await.take() {
                let _ = io.child.kill().await;
            }
        }
        Ok(())
    }

    async fn read_observation(&self) -> Result<Observation> {
        self.request_observation().await
    }

    async fn execute_command(&mut self, cmd: &JointCommand, cancel: &CancelToken) -> Result<()> {
        if cancel.is_cancelled() {
            bail!("cancelled");
        }
        let resp = self
            .send_recv(ClientMsg::Command {
                stamp_us: cmd.stamp_us,
                names: cmd.names.clone(),
                positions: cmd.positions.clone(),
                gripper_open: cmd.gripper_open,
            })
            .await?;
        match resp {
            ServerMsg::Observation { .. } => {
                let obs = resp.into_observation()?;
                *self.last_obs.lock().await = Some(obs);
                Ok(())
            }
            ServerMsg::Ack => Ok(()),
            other => bail!("unexpected command response: {:?}", other),
        }
    }

    async fn estop(&mut self) -> Result<()> {
        let _ = self
            .send_recv(ClientMsg::Estop {
                reason: "executive".into(),
            })
            .await;
        Ok(())
    }
}

/// Resolve default worker script path relative to workspace / CARGO_MANIFEST_DIR ancestors.
pub fn default_worker_script(platform: &str) -> PathBuf {
    let name = match platform {
        "maniskill" => "maniskill_worker.py",
        _ => "mujoco_worker.py",
    };
    let candidates = [
        PathBuf::from("python/archon_sim_workers").join(name),
        PathBuf::from("../python/archon_sim_workers").join(name),
        PathBuf::from("../../python/archon_sim_workers").join(name),
    ];
    for c in candidates {
        if c.exists() {
            return std::fs::canonicalize(&c).unwrap_or(c);
        }
    }
    PathBuf::from("python/archon_sim_workers").join(name)
}

pub fn workspace_python_root() -> Option<PathBuf> {
    let candidates = [
        PathBuf::from("python"),
        PathBuf::from("../python"),
        PathBuf::from("../../python"),
    ];
    for p in candidates {
        if p.join("archon_sim_workers").is_dir() {
            return Some(std::fs::canonicalize(&p).unwrap_or(p));
        }
    }
    None
}

/// Helper for tests / docs.
pub fn ensure_script_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        bail!(
            "worker script not found: {}. Run from repo root or pass --worker.",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::{now_us, CancelToken, JointCommand};

    fn mock_script() -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest.join("../..");
        let script = workspace.join("python/archon_sim_workers/mock_worker.py");
        assert!(
            script.exists(),
            "missing {}",
            script.display()
        );
        script
    }

    #[tokio::test]
    async fn mock_worker_roundtrip() {
        let script = mock_script();
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cfg = BridgeConfig {
            platform: "mock".into(),
            worker_cmd: vec![
                "python3".into(),
                "-u".into(),
                script.to_string_lossy().into_owned(),
            ],
            dof: 6,
            joint_names: (1..=6).map(|i| format!("joint_{i}")).collect(),
            render: false,
            media_root: None,
            cwd: Some(workspace),
            model_path: None,
            camera: None,
            auto_joints: false,
            viewer: false,
            record_dir: None,
            video_out: None,
            record_width: 1280,
            record_height: 720,
        };
        let mut backend = BridgedSimBackend::new(cfg);
        backend.connect().await.expect("connect mock worker");
        let obs = backend.read_observation().await.expect("observe");
        assert_eq!(obs.joints().dof(), 6);
        let cmd = JointCommand {
            stamp_us: now_us(),
            names: (1..=6).map(|i| format!("joint_{i}")).collect(),
            positions: vec![0.1, 0.0, 0.0, 0.0, 0.0, 0.0],
            gripper_open: Some(1.0),
        };
        let cancel = CancelToken::new();
        backend.execute_command(&cmd, &cancel).await.unwrap();
        let obs2 = backend.read_observation().await.unwrap();
        assert!((obs2.joints().positions[0] - 0.1).abs() < 1e-9);
        backend.shutdown().await.unwrap();
    }
}
