//! LLM policy: natural language → structured primitive sequence → joint waypoints.

use anyhow::{Context, Result};
use archon_embodied::{now_us, ActionProposal, Policy, ResourceKind, WorldState};
use async_trait::async_trait;
use serde::Deserialize;

use crate::chat::{extract_json_object, ChatClient};
use crate::instruction::{
    build_waypoint_sequence, MotionPrimitive, RobotKind,
};

#[derive(Debug, Clone)]
pub struct LlmPolicyConfig {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub robot: RobotKind,
    /// If LLM fails, fall back to rule-based InstructionPolicy parse.
    pub fallback_rules: bool,
}

impl LlmPolicyConfig {
    pub fn deepseek(api_key: impl Into<String>, robot: RobotKind) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.deepseek.com".into(),
            model: "deepseek-chat".into(),
            robot,
            fallback_rules: true,
        }
    }
}

pub struct LlmPolicy {
    client: ChatClient,
    robot: RobotKind,
    fallback_rules: bool,
    pub instruction: String,
}

impl LlmPolicy {
    pub fn new(cfg: LlmPolicyConfig, instruction: impl Into<String>) -> Self {
        let client = ChatClient::new(cfg.api_key, cfg.base_url, cfg.model);
        Self {
            client,
            robot: cfg.robot,
            fallback_rules: cfg.fallback_rules,
            instruction: instruction.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LlmStep {
    primitive: String,
    #[serde(default)]
    scale: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct LlmPlan {
    /// Preferred: multi-step plan.
    #[serde(default)]
    steps: Option<Vec<LlmStep>>,
    /// Legacy single-step fields.
    #[serde(default)]
    primitive: Option<String>,
    #[serde(default)]
    scale: Option<f64>,
    #[serde(default)]
    note: Option<String>,
}

#[async_trait]
impl Policy for LlmPolicy {
    fn name(&self) -> &str {
        "llm"
    }

    async fn propose(&self, state: &WorldState) -> Result<ActionProposal> {
        let text = state
            .task_context
            .get("instruction")
            .and_then(|v| v.as_str())
            .unwrap_or(self.instruction.as_str());

        let (steps, source) = match self.compile(text).await {
            Ok((s, note)) => (s, format!("llm:{}", note.unwrap_or_default())),
            Err(e) if self.fallback_rules => {
                let s = MotionPrimitive::parse_sequence_for(text, self.robot)
                    .with_context(|| format!("LLM failed ({e}); rule fallback also failed"))?;
                (s, "rules_fallback".into())
            }
            Err(e) => return Err(e),
        };

        let dof = state.joints().dof().max(1).min(16);
        let start: Vec<f64> = (0..dof)
            .map(|i| state.joints().positions.get(i).copied().unwrap_or(0.0))
            .collect();
        let grip = state.gripper_open();
        let waypoints = build_waypoint_sequence(self.robot, &steps, &start, grip);
        let prim_ids: Vec<&str> = steps.iter().map(|(p, _)| p.id()).collect();

        Ok(ActionProposal {
            id: format!("llm-{}-{}", prim_ids.join("+"), now_us()),
            stamp_us: now_us(),
            source: self.name().into(),
            waypoints,
            confidence: 0.8,
            required_resources: resources_for(self.robot),
            metadata: serde_json::json!({
                "instruction": text,
                "primitives": prim_ids,
                "compiler": source,
                "robot": self.robot.as_str(),
            }),
        })
    }
}

impl LlmPolicy {
    async fn compile(
        &self,
        text: &str,
    ) -> Result<(Vec<(MotionPrimitive, f64)>, Option<String>)> {
        let system = system_prompt(self.robot);
        let user = format!(
            "User instruction:\n{text}\n\nRespond with a single JSON object only."
        );
        let raw = self.client.chat(&system, &user).await?;
        let json = extract_json_object(&raw)?;
        let plan: LlmPlan = serde_json::from_str(&json)
            .with_context(|| format!("invalid plan JSON: {json}"))?;

        let mut steps = Vec::new();
        if let Some(list) = plan.steps {
            for st in list {
                let prim = MotionPrimitive::from_id(&st.primitive, self.robot)
                    .with_context(|| format!("unknown primitive '{}'", st.primitive))?;
                let scale = st.scale.unwrap_or(1.0).clamp(0.2, 4.0);
                steps.push((prim, scale));
            }
        } else if let Some(p) = plan.primitive {
            let prim = MotionPrimitive::from_id(&p, self.robot)
                .with_context(|| format!("unknown primitive '{p}'"))?;
            let scale = plan.scale.unwrap_or(1.0).clamp(0.2, 3.0);
            steps.push((prim, scale));
        }
        if steps.is_empty() {
            anyhow::bail!("LLM plan contained no steps");
        }
        Ok((steps, plan.note))
    }
}

fn resources_for(robot: RobotKind) -> Vec<ResourceKind> {
    match robot {
        RobotKind::Arm => vec![ResourceKind::Arm, ResourceKind::Gripper],
        RobotKind::DiffCar => vec![ResourceKind::Base],
    }
}

fn system_prompt(robot: RobotKind) -> String {
    match robot {
        RobotKind::Arm => r#"You are a robot motion compiler for a 6-DoF desktop arm in MuJoCo.
Map the user instruction to an ORDERED list of primitives from:
home, wave, reach, nod, open_gripper, close_gripper, demo

If the user asks for multiple actions (e.g. wave then open gripper), output multiple steps.
Return JSON only:
{"steps":[{"primitive":"wave","scale":1.0},{"primitive":"open_gripper","scale":1.0}],"note":"short reason"}
scale is optional (0.2–3.0)."#.into(),
        RobotKind::DiffCar => r#"You are a robot motion compiler for a planar mobile base (x, y, yaw) in MuJoCo.
Map the user instruction to an ORDERED list of primitives from:
home, forward, backward, turn_left, turn_right, stop, demo

If the user asks for multiple actions, output multiple steps in order.
If the user specifies an angle like 180 degrees / 半圈, set scale so that scale=1 means 90°, scale=2 means 180°, scale=4 means 360°.
Return JSON only, for example:
{"steps":[{"primitive":"forward","scale":1.0},{"primitive":"turn_left","scale":2.0}],"note":"forward then yaw +180deg"}
forward/backward move in heading direction; turn_left/right change yaw."#.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_fenced_json() {
        let raw = "here you go\n```json\n{\"primitive\":\"wave\",\"scale\":1}\n```\n";
        let j = extract_json_object(raw).unwrap();
        assert!(j.contains("wave"));
    }
}
