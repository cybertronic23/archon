use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Monotonic wall-clock timestamp in microseconds since UNIX epoch.
pub type TimestampUs = u64;

/// Current Observation schema (M1: modalities skeleton).
pub const OBSERVATION_SCHEMA_VERSION: u32 = 1;

/// Current Episode schema.
pub const EPISODE_SCHEMA_VERSION: u32 = 1;

/// Well-known modality keys (LeRobot / OXE-aligned).
pub mod modality_keys {
    pub const IMAGES_PRIMARY: &str = "images.primary";
    pub const IMAGES_WRIST: &str = "images.wrist";
    pub const IMAGES_SECONDARY: &str = "images.secondary";
    pub const DEPTH_PRIMARY: &str = "depth.primary";
}

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

/// Proprioception (high-rate, inline).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProprioState {
    pub joints: JointState,
    #[serde(default)]
    pub gripper_open: f64,
}

impl ProprioState {
    pub fn new(joints: JointState, gripper_open: f64) -> Self {
        Self {
            joints,
            gripper_open,
        }
    }
}

/// How cold media is referenced (pixels/points stay off the hot path).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaStorageKind {
    InlineUri,
    Shm,
    RosTopicHint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaRef {
    pub kind: MediaStorageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_size: Option<u64>,
}

impl MediaRef {
    pub fn uri(uri: impl Into<String>) -> Self {
        Self {
            kind: MediaStorageKind::InlineUri,
            uri: Some(uri.into()),
            topic: None,
            byte_size: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MediaLayout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channels: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

/// One modality sample: metadata + cold storage ref.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModalitySample {
    pub stamp_us: TimestampUs,
    pub frame_id: String,
    pub encoding: String,
    #[serde(default)]
    pub layout: MediaLayout,
    pub storage: MediaRef,
}

/// Derived perception product (not a raw sensor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// e.g. "detection"
    pub kind: String,
    pub stamp_us: TimestampUs,
    /// Optional modality this annotation refers to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modality_key: Option<String>,
    pub payload: serde_json::Value,
}

impl Annotation {
    pub fn detection(
        stamp_us: TimestampUs,
        modality_key: &str,
        label: &str,
        score: f64,
        bbox_xywh: [f64; 4],
    ) -> Self {
        Self {
            kind: "detection".into(),
            stamp_us,
            modality_key: Some(modality_key.into()),
            payload: serde_json::json!({
                "label": label,
                "score": score,
                "bbox_xywh": bbox_xywh,
            }),
        }
    }
}

/// Sensor snapshot at one time step (multimodal).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub stamp_us: TimestampUs,
    #[serde(default = "default_obs_schema")]
    pub schema_version: u32,
    pub proprio: ProprioState,
    #[serde(default)]
    pub modalities: BTreeMap<String, ModalitySample>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

fn default_obs_schema() -> u32 {
    OBSERVATION_SCHEMA_VERSION
}

impl Observation {
    pub fn from_proprio(proprio: ProprioState) -> Self {
        Self {
            stamp_us: now_us(),
            schema_version: OBSERVATION_SCHEMA_VERSION,
            proprio,
            modalities: BTreeMap::new(),
            annotations: Vec::new(),
        }
    }

    pub fn joints(&self) -> &JointState {
        &self.proprio.joints
    }

    pub fn gripper_open(&self) -> f64 {
        self.proprio.gripper_open
    }
}

/// Filtered / estimated world state fed to policies (lightweight view).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldState {
    pub stamp_us: TimestampUs,
    pub proprio: ProprioState,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    #[serde(default)]
    pub modality_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_image_uri: Option<String>,
    /// Free-form task context (goal id, language, etc.).
    #[serde(default)]
    pub task_context: serde_json::Value,
}

impl WorldState {
    pub fn from_observation(obs: &Observation, task_context: serde_json::Value) -> Self {
        let primary_image_uri = obs
            .modalities
            .get(modality_keys::IMAGES_PRIMARY)
            .and_then(|m| m.storage.uri.clone());
        Self {
            stamp_us: obs.stamp_us,
            proprio: obs.proprio.clone(),
            annotations: obs.annotations.clone(),
            modality_keys: obs.modalities.keys().cloned().collect(),
            primary_image_uri,
            task_context,
        }
    }

    pub fn joints(&self) -> &JointState {
        &self.proprio.joints
    }

    pub fn gripper_open(&self) -> f64 {
        self.proprio.gripper_open
    }

    pub fn detections(&self) -> impl Iterator<Item = &Annotation> {
        self.annotations.iter().filter(|a| a.kind == "detection")
    }

    pub fn has_detection_label(&self, label: &str) -> bool {
        self.detections().any(|a| {
            a.payload
                .get("label")
                .and_then(|v| v.as_str())
                .map(|l| l == label)
                .unwrap_or(false)
        })
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
    #[serde(default = "default_episode_schema")]
    pub schema_version: u32,
    pub started_us: TimestampUs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_us: Option<TimestampUs>,
    pub task_id: String,
    pub backend: String,
    #[serde(default)]
    pub events: Vec<EpisodeEvent>,
}

fn default_episode_schema() -> u32 {
    EPISODE_SCHEMA_VERSION
}

impl Episode {
    pub fn new(id: impl Into<String>, task_id: impl Into<String>, backend: impl Into<String>) -> Self {
        let started_us = now_us();
        Self {
            id: id.into(),
            schema_version: EPISODE_SCHEMA_VERSION,
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

    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Save episode.json under a directory (caller may also write media/ beside it).
    pub fn save_bundle(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        self.save_to_file(&dir.join("episode.json"))?;
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
        let js = JointState::new(vec!["j1".into(), "j2".into()], vec![0.1, 0.2]);
        assert_eq!(js.dof(), 2);
    }

    #[test]
    fn observation_modalities_roundtrip() {
        let mut obs = Observation::from_proprio(ProprioState::new(
            JointState::new(vec!["j1".into()], vec![0.0]),
            0.5,
        ));
        obs.modalities.insert(
            modality_keys::IMAGES_PRIMARY.into(),
            ModalitySample {
                stamp_us: 1,
                frame_id: "camera_link".into(),
                encoding: "rgb8".into(),
                layout: MediaLayout {
                    width: Some(8),
                    height: Some(8),
                    channels: Some(3),
                    count: None,
                },
                storage: MediaRef::uri("media/images.primary/000001.ppm"),
            },
        );
        obs.annotations.push(Annotation::detection(
            1,
            modality_keys::IMAGES_PRIMARY,
            "red_blob",
            0.9,
            [1.0, 2.0, 3.0, 4.0],
        ));
        let s = serde_json::to_string(&obs).unwrap();
        let back: Observation = serde_json::from_str(&s).unwrap();
        assert!(back.modalities.contains_key(modality_keys::IMAGES_PRIMARY));
        assert_eq!(back.annotations.len(), 1);
    }

    #[test]
    fn world_state_detects_label() {
        let mut obs = Observation::from_proprio(ProprioState::new(
            JointState::new(vec![], vec![]),
            0.0,
        ));
        obs.annotations.push(Annotation::detection(
            0,
            modality_keys::IMAGES_PRIMARY,
            "red_blob",
            1.0,
            [0.0, 0.0, 1.0, 1.0],
        ));
        let ws = WorldState::from_observation(&obs, serde_json::json!({}));
        assert!(ws.has_detection_label("red_blob"));
        assert!(!ws.has_detection_label("blue_blob"));
    }

    #[test]
    fn episode_roundtrip_json() {
        let mut ep = Episode::new("ep1", "demo", "sim");
        ep.push("observation", serde_json::json!({ "n": 1 }));
        ep.finish();
        let s = serde_json::to_string(&ep).unwrap();
        let back: Episode = serde_json::from_str(&s).unwrap();
        assert_eq!(back.events.len(), 1);
        assert_eq!(back.schema_version, EPISODE_SCHEMA_VERSION);
        assert!(back.ended_us.is_some());
    }
}
