//! Vision-gated policy: act only when a color blob is annotated.

use anyhow::Result;
use async_trait::async_trait;
use archon_embodied::{
    now_us, ActionProposal, JointWaypoint, Policy, ResourceKind, WorldState,
};

/// Proposes a simplified pick-place primitive when `red_blob` (or configured label)
/// is present in WorldState annotations. Otherwise returns an empty/rejected-style
/// proposal with confidence 0 (Executive/Safety should not move).
pub struct ColorBlobPolicy {
    pub joint_names: Vec<String>,
    pub target_label: String,
}

impl Default for ColorBlobPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorBlobPolicy {
    pub fn new() -> Self {
        Self {
            joint_names: (1..=6).map(|i| format!("joint_{i}")).collect(),
            target_label: "red_blob".into(),
        }
    }

    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            target_label: label.into(),
            ..Self::new()
        }
    }
}

#[async_trait]
impl Policy for ColorBlobPolicy {
    fn name(&self) -> &str {
        "color_blob"
    }

    async fn propose(&self, state: &WorldState) -> Result<ActionProposal> {
        if !state.has_detection_label(&self.target_label) {
            return Ok(ActionProposal {
                id: format!("blob-miss-{}", now_us()),
                stamp_us: now_us(),
                source: self.name().into(),
                waypoints: Vec::new(),
                confidence: 0.0,
                required_resources: vec![],
                metadata: serde_json::json!({
                    "reason": "target_not_detected",
                    "expected_label": self.target_label,
                    "modality_keys": state.modality_keys,
                }),
            });
        }

        let dof = state.joints().dof().max(self.joint_names.len()).min(6);
        let start: Vec<f64> = (0..dof)
            .map(|i| state.joints().positions.get(i).copied().unwrap_or(0.0))
            .collect();

        // Simplified approach → grasp → retreat (joint-space heuristic, not IK).
        let approach: Vec<f64> = start
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let delta = match i {
                    0 => 0.15,
                    1 => 0.35,
                    2 => -0.2,
                    _ => 0.05,
                };
                (p + delta).clamp(-2.0, 2.0)
            })
            .collect();

        let grasp = approach.clone();
        let retreat: Vec<f64> = start
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let delta = if i == 1 { 0.1 } else { 0.0 };
                (p + delta).clamp(-2.0, 2.0)
            })
            .collect();

        Ok(ActionProposal {
            id: format!("blob-{}", now_us()),
            stamp_us: now_us(),
            source: self.name().into(),
            waypoints: vec![
                JointWaypoint {
                    t_sec: 0.0,
                    positions: start,
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 0.8,
                    positions: approach,
                    gripper_open: Some(1.0),
                },
                JointWaypoint {
                    t_sec: 1.2,
                    positions: grasp,
                    gripper_open: Some(0.0),
                },
                JointWaypoint {
                    t_sec: 2.0,
                    positions: retreat,
                    gripper_open: Some(0.0),
                },
            ],
            confidence: 0.85,
            required_resources: vec![ResourceKind::Arm, ResourceKind::Gripper],
            metadata: serde_json::json!({
                "target_label": self.target_label,
                "primary_image_uri": state.primary_image_uri,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archon_embodied::{Annotation, JointState, ProprioState, modality_keys};

    fn state_with_blob(has: bool) -> WorldState {
        let mut annotations = vec![];
        if has {
            annotations.push(Annotation::detection(
                0,
                modality_keys::IMAGES_PRIMARY,
                "red_blob",
                0.9,
                [10.0, 10.0, 8.0, 8.0],
            ));
        }
        WorldState {
            stamp_us: 0,
            proprio: ProprioState::new(
                JointState::new((1..=6).map(|i| format!("joint_{i}")).collect(), vec![0.0; 6]),
                0.5,
            ),
            annotations,
            modality_keys: if has {
                vec![modality_keys::IMAGES_PRIMARY.into()]
            } else {
                vec![]
            },
            primary_image_uri: None,
            task_context: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn proposes_when_blob_present() {
        let policy = ColorBlobPolicy::new();
        let p = policy.propose(&state_with_blob(true)).await.unwrap();
        assert_eq!(p.waypoints.len(), 4);
        assert!(p.confidence > 0.5);
    }

    #[tokio::test]
    async fn empty_when_blob_missing() {
        let policy = ColorBlobPolicy::new();
        let p = policy.propose(&state_with_blob(false)).await.unwrap();
        assert!(p.waypoints.is_empty());
        assert_eq!(p.confidence, 0.0);
    }
}
