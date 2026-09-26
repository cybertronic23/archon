//! Deterministic joint-limit and timeout safety gate.

use archon_embodied::{
    ActionProposal, JointCommand, SafetyGate, SafetyVerdict, WorldState,
};
use archon_kinetic::JointLimits;
use async_trait::async_trait;

pub struct LimitSafetyGate {
    pub limits: JointLimits,
    pub max_waypoint_t_sec: f64,
    pub max_commands_hint: usize,
}

impl Default for LimitSafetyGate {
    fn default() -> Self {
        Self {
            limits: JointLimits::desktop_arm_6dof(),
            max_waypoint_t_sec: 30.0,
            max_commands_hint: 50 * 60, // 1 min @ 50Hz
        }
    }
}

impl LimitSafetyGate {
    pub fn new(limits: JointLimits) -> Self {
        Self {
            limits,
            ..Self::default()
        }
    }
}

#[async_trait]
impl SafetyGate for LimitSafetyGate {
    async fn check_proposal(&self, proposal: &ActionProposal, _state: &WorldState) -> SafetyVerdict {
        if proposal.waypoints.is_empty() {
            return SafetyVerdict::Deny {
                reason: "empty waypoints".into(),
            };
        }
        for wp in &proposal.waypoints {
            if wp.t_sec < 0.0 || wp.t_sec > self.max_waypoint_t_sec {
                return SafetyVerdict::Deny {
                    reason: format!(
                        "waypoint t_sec={:.3} outside [0, {}]",
                        wp.t_sec, self.max_waypoint_t_sec
                    ),
                };
            }
            if let Err(e) = self.limits.contains(&wp.positions) {
                return SafetyVerdict::Deny { reason: e };
            }
            if let Some(g) = wp.gripper_open {
                if !(0.0..=1.0).contains(&g) {
                    return SafetyVerdict::Deny {
                        reason: format!("gripper_open={g} outside [0, 1]"),
                    };
                }
            }
        }
        SafetyVerdict::Allow
    }

    async fn check_command(&self, cmd: &JointCommand, _state: &WorldState) -> SafetyVerdict {
        if let Err(e) = self.limits.contains(&cmd.positions) {
            return SafetyVerdict::Deny { reason: e };
        }
        if let Some(g) = cmd.gripper_open {
            if !(0.0..=1.0).contains(&g) {
                return SafetyVerdict::Deny {
                    reason: format!("gripper_open={g} outside [0, 1]"),
                };
            }
        }
        SafetyVerdict::Allow
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::{now_us, JointWaypoint, ResourceKind};

    #[tokio::test]
    async fn denies_oob_waypoint() {
        let gate = LimitSafetyGate::default();
        let proposal = ActionProposal {
            id: "bad".into(),
            stamp_us: now_us(),
            source: "test".into(),
            waypoints: vec![JointWaypoint {
                t_sec: 0.0,
                positions: vec![9.0; 6],
                gripper_open: Some(0.5),
            }],
            confidence: 1.0,
            required_resources: vec![ResourceKind::Arm],
            metadata: serde_json::json!({}),
        };
        let state = WorldState {
            stamp_us: 0,
            joints: archon_embodied::JointState::new(vec![], vec![]),
            gripper_open: 0.0,
            task_context: serde_json::json!({}),
        };
        match gate.check_proposal(&proposal, &state).await {
            SafetyVerdict::Deny { .. } => {}
            SafetyVerdict::Allow => panic!("expected deny"),
        }
    }
}
