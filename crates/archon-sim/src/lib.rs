//! In-process desktop-arm simulator implementing `RobotBackend`.
//!
//! Uses the same logical topic contract as [`archon_ros2::TopicContract`] so a
//! future ROS2 bridge can swap in without changing the Executive.

use std::sync::Arc;

use anyhow::{bail, Result};
use archon_embodied::{
    now_us, CancelToken, JointCommand, JointState, Observation, RobotBackend,
};
use archon_ros2::{messages::JointStateMsg, TopicContract};
use async_trait::async_trait;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct SimConfig {
    pub dof: usize,
    pub joint_names: Vec<String>,
    /// Seconds of simulated motion settle per command (wall-clock sleep scaled down).
    pub settle_ms: u64,
    pub topics: TopicContract,
}

impl Default for SimConfig {
    fn default() -> Self {
        let dof = 6;
        Self {
            dof,
            joint_names: (1..=dof).map(|i| format!("joint_{i}")).collect(),
            settle_ms: 0,
            topics: TopicContract::default(),
        }
    }
}

struct SimInner {
    connected: bool,
    estop: bool,
    joints: JointState,
    gripper_open: f64,
    /// Last published joint_states message (contract mirror).
    last_joint_states_msg: Option<JointStateMsg>,
}

/// Process-local simulator — no MuJoCo/Gazebo process required for MVP.
pub struct SimBackend {
    config: SimConfig,
    inner: Arc<Mutex<SimInner>>,
}

impl SimBackend {
    pub fn new(config: SimConfig) -> Self {
        let joints = JointState::new(config.joint_names.clone(), vec![0.0; config.dof]);
        Self {
            config,
            inner: Arc::new(Mutex::new(SimInner {
                connected: false,
                estop: false,
                joints,
                gripper_open: 0.0,
                last_joint_states_msg: None,
            })),
        }
    }

    pub fn desktop_arm() -> Self {
        Self::new(SimConfig::default())
    }

    pub fn topic_contract(&self) -> &TopicContract {
        &self.config.topics
    }

    /// Snapshot of the mirrored `/joint_states` message for contract tests.
    pub async fn last_joint_states_msg(&self) -> Option<JointStateMsg> {
        self.inner.lock().await.last_joint_states_msg.clone()
    }
}

#[async_trait]
impl RobotBackend for SimBackend {
    fn name(&self) -> &str {
        "sim"
    }

    async fn connect(&mut self) -> Result<()> {
        let mut g = self.inner.lock().await;
        g.connected = true;
        g.estop = false;
        eprintln!(
            "[archon-sim] connected joint_states={} joint_command={}",
            self.config.topics.joint_states(),
            self.config.topics.joint_command()
        );
        Ok(())
    }

    async fn shutdown(&mut self) -> Result<()> {
        let mut g = self.inner.lock().await;
        g.connected = false;
        Ok(())
    }

    async fn read_observation(&self) -> Result<Observation> {
        let g = self.inner.lock().await;
        if !g.connected {
            bail!("sim backend not connected");
        }
        Ok(Observation {
            stamp_us: now_us(),
            joints: g.joints.clone(),
            gripper_open: g.gripper_open,
            image: None,
        })
    }

    async fn execute_command(&mut self, cmd: &JointCommand, cancel: &CancelToken) -> Result<()> {
        if cancel.is_cancelled() {
            bail!("cancelled before execute");
        }
        let mut g = self.inner.lock().await;
        if !g.connected {
            bail!("sim backend not connected");
        }
        if g.estop {
            bail!("estop active");
        }

        let n = cmd.positions.len().min(g.joints.positions.len());
        for i in 0..n {
            g.joints.positions[i] = cmd.positions[i];
        }
        if let Some(grip) = cmd.gripper_open {
            g.gripper_open = grip.clamp(0.0, 1.0);
        }
        g.joints.names = if cmd.names.is_empty() {
            self.config.joint_names.clone()
        } else {
            cmd.names.clone()
        };

        let msg = JointStateMsg {
            name: g.joints.names.clone(),
            position: g.joints.positions.clone(),
            velocity: g.joints.velocities.clone(),
            stamp_ns: now_us().saturating_mul(1000),
        };
        g.last_joint_states_msg = Some(msg);

        drop(g);
        if self.config.settle_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.config.settle_ms)).await;
        }
        Ok(())
    }

    async fn estop(&mut self) -> Result<()> {
        let mut g = self.inner.lock().await;
        g.estop = true;
        eprintln!(
            "[archon-sim] estop latched on {}",
            self.config.topics.estop()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::RobotBackendExt;

    #[tokio::test]
    async fn moves_to_command() {
        let mut sim = SimBackend::desktop_arm();
        sim.connect().await.unwrap();
        let cmd = JointCommand {
            stamp_us: now_us(),
            names: sim.config.joint_names.clone(),
            positions: vec![0.1, 0.2, 0.0, 0.0, 0.0, 0.0],
            gripper_open: Some(0.8),
        };
        let cancel = CancelToken::new();
        sim.execute_command(&cmd, &cancel).await.unwrap();
        let obs = sim.read_observation().await.unwrap();
        assert!((obs.joints.positions[0] - 0.1).abs() < 1e-9);
        assert!((obs.gripper_open - 0.8).abs() < 1e-9);
        assert!(sim.last_joint_states_msg().await.is_some());
    }

    #[tokio::test]
    async fn stream_completes() {
        let mut sim = SimBackend::desktop_arm();
        sim.connect().await.unwrap();
        let cmds = vec![
            JointCommand {
                stamp_us: now_us(),
                names: vec![],
                positions: vec![0.0; 6],
                gripper_open: Some(1.0),
            },
            JointCommand {
                stamp_us: now_us(),
                names: vec![],
                positions: vec![0.1; 6],
                gripper_open: Some(0.0),
            },
        ];
        let cancel = CancelToken::new();
        let r = sim.execute_stream(&cmds, &cancel, 0).await.unwrap();
        assert_eq!(r.commands_sent, 2);
    }
}
