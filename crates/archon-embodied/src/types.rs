use serde::{Deserialize, Serialize};

/// Monotonic wall-clock timestamp in microseconds since UNIX epoch.
pub type TimestampUs = u64;

/// Named robot resource that can be locked by the executive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Arm,
    Gripper,
    Base,
    Speaker,
    Camera,
}

/// Joint positions (radians) for a multi-DoF arm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointState {
    pub names: Vec<String>,
    pub positions: Vec<f64>,
    #[serde(default)]
    pub velocities: Vec<f64>,
}

impl JointState {
    pub fn new(names: Vec<String>, positions: Vec<f64>) -> Self {
        Self {
            names,
            positions,
            velocities: Vec::new(),
        }
    }

    pub fn dof(&self) -> usize {
        self.positions.len()
    }
}

/// Optional RGB frame summary (full pixels deferred to later milestones).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageSummary {
    pub width: u32,
    pub height: u32,
    pub encoding: String,
    /// Optional path or URI if frame was persisted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// Sensor snapshot at one time step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub stamp_us: TimestampUs,
    pub joints: JointState,
    #[serde(default)]
    pub gripper_open: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageSummary>,
}

/// Filtered / estimated world state fed to policies.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldState {
    pub stamp_us: TimestampUs,
    pub joints: JointState,
    pub gripper_open: f64,
    /// Free-form task context (goal id, language, etc.).
    #[serde(default)]
    pub task_context: serde_json::Value,
}

impl WorldState {
    pub fn from_observation(obs: &Observation, task_context: serde_json::Value) -> Self {
        Self {
            stamp_us: obs.stamp_us,
            joints: obs.joints.clone(),
            gripper_open: obs.gripper_open,
            task_context,
        }
    }
}

/// High-rate command sent to the robot backend after Chronos interpolation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointCommand {
    pub stamp_us: TimestampUs,
    pub names: Vec<String>,
    pub positions: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gripper_open: Option<f64>,
}

/// Sparse waypoint used in action proposals before interpolation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointWaypoint {
    /// Time offset from proposal start, seconds.
    pub t_sec: f64,
    pub positions: Vec<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gripper_open: Option<f64>,
}

/// Model-agnostic action proposal (VLA chunk, planner output, mock policy, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionProposal {
    pub id: String,
    pub stamp_us: TimestampUs,
    pub source: String,
    pub waypoints: Vec<JointWaypoint>,
    /// Confidence in [0, 1] when available.
    #[serde(default)]
    pub confidence: f64,
    #[serde(default)]
    pub required_resources: Vec<ResourceKind>,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

impl ActionProposal {
    pub fn default_resources() -> Vec<ResourceKind> {
        vec![ResourceKind::Arm, ResourceKind::Gripper]
    }
}

/// Outcome of attempting to execute (or reject) a proposal / command stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionStatus {
    Completed,
    Cancelled,
    TimedOut,
    Rejected,
    Fault,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub message: String,
    pub commands_sent: usize,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_joints: Option<JointState>,
}

impl ExecutionResult {
    pub fn completed(commands_sent: usize, duration_ms: u64, final_joints: JointState) -> Self {
        Self {
            status: ExecutionStatus::Completed,
            message: "ok".into(),
            commands_sent,
            duration_ms,
            final_joints: Some(final_joints),
        }
    }

    pub fn cancelled(commands_sent: usize, duration_ms: u64, reason: impl Into<String>) -> Self {
        Self {
            status: ExecutionStatus::Cancelled,
            message: reason.into(),
            commands_sent,
            duration_ms,
            final_joints: None,
        }
    }

    pub fn rejected(reason: impl Into<String>) -> Self {
        Self {
            status: ExecutionStatus::Rejected,
            message: reason.into(),
            commands_sent: 0,
            duration_ms: 0,
            final_joints: None,
        }
    }

    pub fn timed_out(commands_sent: usize, duration_ms: u64) -> Self {
        Self {
            status: ExecutionStatus::TimedOut,
            message: "execution timed out".into(),
            commands_sent,
            duration_ms,
            final_joints: None,
        }
    }
}

/// Safety gate decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafetyVerdict {
    Allow,
    Deny { reason: String },
}

/// One event on the episode timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeEvent {
    pub stamp_us: TimestampUs,
    pub kind: String,
    pub payload: serde_json::Value,
}

/// Full episode log for data flywheel / replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub id: String,
    pub started_us: TimestampUs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_us: Option<TimestampUs>,
    pub task_id: String,
    pub backend: String,
    #[serde(default)]
    pub events: Vec<EpisodeEvent>,
}

impl Episode {
    pub fn new(id: impl Into<String>, task_id: impl Into<String>, backend: impl Into<String>) -> Self {
        let started_us = now_us();
        Self {
            id: id.into(),
            started_us,
            ended_us: None,
            task_id: task_id.into(),
            backend: backend.into(),
            events: Vec::new(),
        }
    }

    pub fn push(&mut self, kind: impl Into<String>, payload: serde_json::Value) {
        self.events.push(EpisodeEvent {
            stamp_us: now_us(),
            kind: kind.into(),
            payload,
        });
    }

    pub fn finish(&mut self) {
        self.ended_us = Some(now_us());
    }

    pub fn save_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

pub fn now_us() -> TimestampUs {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_state_dof() {
        let js = JointState::new(
            vec!["j1".into(), "j2".into()],
            vec![0.1, 0.2],
        );
        assert_eq!(js.dof(), 2);
    }

    #[test]
    fn episode_roundtrip_json() {
        let mut ep = Episode::new("ep1", "demo", "sim");
        ep.push("observation", serde_json::json!({"n": 1}));
        ep.finish();
        let s = serde_json::to_string(&ep).unwrap();
        let back: Episode = serde_json::from_str(&s).unwrap();
        assert_eq!(back.events.len(), 1);
        assert!(back.ended_us.is_some());
    }
}
