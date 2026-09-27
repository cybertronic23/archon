//! Multi-turn execution helper shared by one-shot CLI and TUI.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use archon_embodied::{Episode, ExecutionResult, Policy, SafetyGate};
use archon_perception::PerceptionBridge;
use archon_policy::{
    InstructionPolicy, LimitSafetyGate, LlmPolicy, LlmPolicyConfig, MockPolicy, RobotKind,
};
use archon_runtime::Executive;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SessionConfig {
    pub task_id: String,
    pub backend_name: String,
    pub policy_name: String,
    pub camera: String,
    pub model: String,
    pub robot: RobotKind,
    pub joint_names: Vec<String>,
    pub episode_root: PathBuf,
    pub save_frames: bool,
    pub llm_api_key: Option<String>,
    pub llm_base_url: Option<String>,
    pub llm_model: String,
}

pub struct TurnOutcome {
    pub result: ExecutionResult,
    #[allow(dead_code)]
    pub episode: Episode,
    pub episode_path: PathBuf,
}

pub async fn run_turn(
    executive: &mut Executive,
    backend: Arc<Mutex<dyn archon_embodied::RobotBackend>>,
    safety: &dyn SafetyGate,
    perception: &mut Option<PerceptionBridge>,
    cfg: &SessionConfig,
    instruction: &str,
) -> Result<TurnOutcome> {
    let episode_id = format!("ep-{}", archon_embodied::now_us());
    executive.config.episode_id = Some(episode_id.clone());
    let bundle_dir = cfg.episode_root.join(&episode_id);

    let task_context = serde_json::json!({
        "task_id": cfg.task_id,
        "backend": cfg.backend_name,
        "policy": cfg.policy_name,
        "camera": cfg.camera,
        "model": cfg.model,
        "instruction": instruction,
        "robot": cfg.robot.as_str(),
    });

    let (result, episode) = match cfg.policy_name.as_str() {
        "mock" => {
            let policy = MockPolicy::new();
            run_with_optional_perception(
                executive,
                &policy,
                safety,
                backend,
                perception.as_mut(),
                task_context,
            )
            .await?
        }
        "instruction" => {
            let policy = InstructionPolicy::new(instruction)
                .with_joint_names(cfg.joint_names.clone())
                .with_robot(cfg.robot);
            run_with_optional_perception(
                executive,
                &policy,
                safety,
                backend,
                perception.as_mut(),
                task_context,
            )
            .await?
        }
        "llm" => {
            let api_key = cfg
                .llm_api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .context(
                    "LLM policy needs DEEPSEEK_API_KEY or OPENAI_API_KEY (or --llm-api-key)",
                )?;
            let base_url = cfg
                .llm_base_url
                .clone()
                .unwrap_or_else(|| "https://api.deepseek.com".into());
            let llm_cfg = LlmPolicyConfig {
                api_key,
                base_url,
                model: cfg.llm_model.clone(),
                robot: cfg.robot,
                fallback_rules: true,
            };
            let policy = LlmPolicy::new(llm_cfg, instruction);
            run_with_optional_perception(
                executive,
                &policy,
                safety,
                backend,
                perception.as_mut(),
                task_context,
            )
            .await?
        }
        other => anyhow::bail!("TUI/session does not support policy '{other}'"),
    };

    let bridged = matches!(cfg.backend_name.as_str(), "mujoco" | "maniskill");
    let episode_path = if (perception.is_some() || bridged) && cfg.save_frames {
        episode
            .save_bundle(&bundle_dir)
            .with_context(|| format!("save episode bundle to {}", bundle_dir.display()))?;
        bundle_dir.join("episode.json")
    } else {
        let path = cfg.episode_root.join(format!("{}.json", episode.id));
        episode
            .save_to_file(&path)
            .with_context(|| format!("save episode to {}", path.display()))?;
        path
    };

    Ok(TurnOutcome {
        result,
        episode,
        episode_path,
    })
}

pub fn default_safety(robot: RobotKind) -> LimitSafetyGate {
    match robot {
        RobotKind::DiffCar => LimitSafetyGate::for_planar_base(),
        RobotKind::Arm => LimitSafetyGate::default(),
    }
}

async fn run_with_optional_perception(
    executive: &mut Executive,
    policy: &dyn Policy,
    safety: &dyn SafetyGate,
    backend: Arc<Mutex<dyn archon_embodied::RobotBackend>>,
    perception: Option<&mut PerceptionBridge>,
    task_context: serde_json::Value,
) -> Result<(ExecutionResult, Episode)> {
    if let Some(bridge) = perception {
        executive
            .run_once_with_perception(policy, safety, backend, Some(bridge), task_context)
            .await
    } else {
        executive
            .run_once(policy, safety, backend, task_context)
            .await
    }
}
