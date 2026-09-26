//! Stable ROS2 topic names shared by sim and real backends.
//!
//! Archon does not bind to a specific simulator API; both MuJoCo/Gazebo bridges
//! and SO101 drivers should publish/subscribe these names (or remaps).

/// Default namespace prefix for a single desktop arm.
pub const DEFAULT_NS: &str = "/archon/arm";

#[derive(Debug, Clone)]
pub struct TopicContract {
    pub namespace: String,
}

impl Default for TopicContract {
    fn default() -> Self {
        Self {
            namespace: DEFAULT_NS.into(),
        }
    }
}

impl TopicContract {
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
        }
    }

    pub fn joint_states(&self) -> String {
        format!("{}/joint_states", self.namespace)
    }

    pub fn joint_command(&self) -> String {
        format!("{}/joint_command", self.namespace)
    }

    pub fn gripper_command(&self) -> String {
        format!("{}/gripper_command", self.namespace)
    }

    pub fn estop(&self) -> String {
        format!("{}/estop", self.namespace)
    }

    pub fn camera_image(&self) -> String {
        format!("{}/camera/image_raw", self.namespace)
    }

    pub fn user_stop(&self) -> String {
        format!("{}/user_stop", self.namespace)
    }
}

/// Documented message shapes (JSON-compatible) mirroring common ROS2 fields.
/// Real r2r bindings can map these 1:1 in a later milestone.
pub mod messages {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct JointStateMsg {
        pub name: Vec<String>,
        pub position: Vec<f64>,
        #[serde(default)]
        pub velocity: Vec<f64>,
        /// ROS time as nanoseconds since epoch (simplified).
        pub stamp_ns: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct JointCommandMsg {
        pub name: Vec<String>,
        pub position: Vec<f64>,
        pub stamp_ns: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct GripperCommandMsg {
        /// 0.0 closed … 1.0 open
        pub position: f64,
        pub stamp_ns: u64,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct EStopMsg {
        pub active: bool,
        pub reason: String,
    }
}

use archon_embodied::{JointCommand, JointState, Observation, ProprioState, now_us};
use messages::{GripperCommandMsg, JointCommandMsg, JointStateMsg};

impl From<&Observation> for JointStateMsg {
    fn from(obs: &Observation) -> Self {
        Self {
            name: obs.joints().names.clone(),
            position: obs.joints().positions.clone(),
            velocity: obs.joints().velocities.clone(),
            stamp_ns: obs.stamp_us.saturating_mul(1000),
        }
    }
}

impl From<&JointCommand> for JointCommandMsg {
    fn from(cmd: &JointCommand) -> Self {
        Self {
            name: cmd.names.clone(),
            position: cmd.positions.clone(),
            stamp_ns: cmd.stamp_us.saturating_mul(1000),
        }
    }
}

pub fn observation_from_joint_state(msg: &JointStateMsg, gripper_open: f64) -> Observation {
    let mut obs = Observation::from_proprio(ProprioState::new(
        JointState {
            names: msg.name.clone(),
            positions: msg.position.clone(),
            velocities: msg.velocity.clone(),
        },
        gripper_open,
    ));
    obs.stamp_us = msg.stamp_ns / 1000;
    obs
}

pub fn gripper_msg(open: f64) -> GripperCommandMsg {
    GripperCommandMsg {
        position: open.clamp(0.0, 1.0),
        stamp_ns: now_us().saturating_mul(1000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_names_stable() {
        let c = TopicContract::default();
        assert_eq!(c.joint_states(), "/archon/arm/joint_states");
        assert_eq!(c.estop(), "/archon/arm/estop");
    }
}
