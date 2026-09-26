//! Mock policy: emits a safe joint waypoint sequence for harness validation.

use anyhow::Result;
use archon_embodied::{
    now_us, ActionProposal, JointWaypoint, Policy, ResourceKind, WorldState,
};
use async_trait::async_trait;

/// Emits a short, in-limits arm motion + gripper open/close for MVP demos.
pub struct MockPolicy {
    pub joint_names: Vec<String>,
}

impl Default for MockPolicy {
    fn default() -> Self {
        Self {
            joint_names: (1..=6).map(|i| format!("joint_{i}")).collect(),
        }
    }
}

impl MockPolicy {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Policy for MockPolicy {
    fn name(&self) -> &str {
        "mock_policy"
    }

    async fn propose(&self, state: &WorldState) -> Result<ActionProposal> {
        let dof = state.joints.dof().max(self.joint_names.len()).min(6);
        let start: Vec<f64> = (0..dof)
            .map(|i| state.joints.positions.get(i).copied().unwrap_or(0.0))
            .collect();

        // Small excursion within ±0.4 rad of current pose.
        let mid: Vec<f64> = start
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let delta = if i % 2 == 0 { 0.3 } else { -0.25 };
                (p + delta).clamp(-2.0, 2.0)
            })
            .collect();

        let waypoints = vec![
            JointWaypoint {
                t_sec: 0.0,
                positions: start.clone(),
                gripper_open: Some(state.gripper_open),
            },
            JointWaypoint {
                t_sec: 1.0,
                positions: mid.clone(),
                gripper_open: Some(1.0),
            },
            JointWaypoint {
                t_sec: 2.0,
                positions: start,
                gripper_open: Some(0.0),
            },
        ];

        Ok(ActionProposal {
            id: format!("mock-{}", now_us()),
            stamp_us: now_us(),
            source: self.name().into(),
            waypoints,
            confidence: 1.0,
            required_resources: vec![ResourceKind::Arm, ResourceKind::Gripper],
            metadata: serde_json::json!({ "dof": dof }),
        })
    }
}

/// Placeholder for future VLA adapters (Cloud / PyO3).
pub struct VlaAdapterStub;

#[async_trait]
impl Policy for VlaAdapterStub {
    fn name(&self) -> &str {
        "vla_adapter_stub"
    }

    async fn propose(&self, _state: &WorldState) -> Result<ActionProposal> {
        anyhow::bail!("VlaAdapter not implemented yet — use MockPolicy for MVP")
    }
}

/// Placeholder for future WAM / world-model adapters.
pub struct WamAdapterStub;

#[async_trait]
impl Policy for WamAdapterStub {
    fn name(&self) -> &str {
        "wam_adapter_stub"
    }

    async fn propose(&self, _state: &WorldState) -> Result<ActionProposal> {
        anyhow::bail!("WamAdapter not implemented yet — use MockPolicy for MVP")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::JointState;

    #[tokio::test]
    async fn mock_emits_three_waypoints() {
        let policy = MockPolicy::new();
        let state = WorldState {
            stamp_us: 0,
            joints: JointState::new(
                policy.joint_names.clone(),
                vec![0.0; 6],
            ),
            gripper_open: 0.5,
            task_context: serde_json::json!({}),
        };
        let p = policy.propose(&state).await.unwrap();
        assert_eq!(p.waypoints.len(), 3);
    }
}
