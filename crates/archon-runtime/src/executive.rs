//! Executive: single authority that runs the embodied control loop.

use std::sync::Arc;

use anyhow::{Context, Result};
use archon_embodied::{
    CancelToken, Episode, ExecutionResult, ExecutionStatus, Observation, Policy, RobotBackend,
    RobotBackendExt, SafetyGate, SafetyVerdict, WorldState,
};
use archon_kinetic::Chronos;
use archon_perception::PerceptionBridge;
use tokio::sync::Mutex;

use crate::arbiter::{Arbiter, ArbiterAction};
use crate::event_bus::{EventBus, RuntimeEvent};
use crate::resource_lock::ResourceLocks;

pub struct ExecutiveConfig {
    pub task_id: String,
    pub control_step_ms: u64,
    pub max_proposal_timeout_ms: u64,
    /// Optional fixed episode id (so CLI can pre-create media dirs).
    pub episode_id: Option<String>,
}

impl Default for ExecutiveConfig {
    fn default() -> Self {
        Self {
            task_id: "demo_waypoints".into(),
            control_step_ms: 20, // 50 Hz
            max_proposal_timeout_ms: 30_000,
            episode_id: None,
        }
    }
}

/// Single execution authority for one robot.
pub struct Executive {
    pub config: ExecutiveConfig,
    pub events: EventBus,
    pub locks: ResourceLocks,
    pub cancel: CancelToken,
    chronos: Chronos,
    arbiter: Arbiter,
}

impl Executive {
    pub fn new(config: ExecutiveConfig, chronos: Chronos) -> Self {
        Self {
            config,
            events: EventBus::new(64),
            locks: ResourceLocks::new(),
            cancel: CancelToken::new(),
            chronos,
            arbiter: Arbiter::new(),
        }
    }

    pub fn request_stop(&self, reason: impl Into<String>) {
        self.events.publish(RuntimeEvent::UserStop {
            reason: reason.into(),
        });
    }

    pub fn request_estop(&self, reason: impl Into<String>) {
        self.events.publish(RuntimeEvent::EStop {
            reason: reason.into(),
        });
    }

    /// Run one embodied cycle: observe → [enrich] → propose → safety → interpolate → execute.
    pub async fn run_once(
        &mut self,
        policy: &dyn Policy,
        safety: &dyn SafetyGate,
        backend: Arc<Mutex<dyn RobotBackend>>,
        task_context: serde_json::Value,
    ) -> Result<(ExecutionResult, Episode)> {
        self.run_once_with_perception(policy, safety, backend, None, task_context)
            .await
    }

    /// Same as `run_once`, optionally enriching proprio with perception modalities.
    pub async fn run_once_with_perception(
        &mut self,
        policy: &dyn Policy,
        safety: &dyn SafetyGate,
        backend: Arc<Mutex<dyn RobotBackend>>,
        mut perception: Option<&mut PerceptionBridge>,
        task_context: serde_json::Value,
    ) -> Result<(ExecutionResult, Episode)> {
        self.cancel.reset();
        self.arbiter.reset();

        let backend_name = {
            let b = backend.lock().await;
            b.name().to_string()
        };

        let episode_id = self
            .config
            .episode_id
            .clone()
            .unwrap_or_else(|| format!("ep-{}", archon_embodied::now_us()));
        let mut episode = Episode::new(&episode_id, &self.config.task_id, &backend_name);

        // Spawn event watcher that cancels on preempt events.
        let mut rx = self.events.subscribe();
        let cancel = self.cancel.clone();
        let mut arbiter = Arbiter::new();
        let watch = tokio::spawn(async move {
            while let Ok(ev) = rx.recv().await {
                let action = arbiter.apply(&ev, &cancel);
                if matches!(action, ArbiterAction::EStop | ArbiterAction::CancelTask) {
                    break;
                }
            }
        });

        // Connect
        {
            let mut b = backend.lock().await;
            b.connect().await.context("backend connect")?;
        }

        let body = {
            let b = backend.lock().await;
            b.read_observation().await.context("read observation")?
        };

        let obs: Observation = if let Some(bridge) = perception.as_mut() {
            let enriched = bridge
                .enrich(body)
                .await
                .context("perception enrich")?;
            episode.push(
                "perception",
                serde_json::json!({
                    "camera": bridge.camera_name(),
                    "modality_keys": enriched.modalities.keys().cloned().collect::<Vec<_>>(),
                    "annotations": enriched.annotations.len(),
                }),
            );
            enriched
        } else {
            body
        };

        episode.push(
            "observation",
            serde_json::to_value(&obs).unwrap_or_default(),
        );

        let state = WorldState::from_observation(&obs, task_context);

        let proposal = policy
            .propose(&state)
            .await
            .context("policy propose")?;
        episode.push(
            "proposal",
            serde_json::to_value(&proposal).unwrap_or_default(),
        );

        let resources = if proposal.required_resources.is_empty() {
            // Empty resources: skip lock acquisition (e.g. vision miss with no motion).
            if proposal.waypoints.is_empty() {
                vec![]
            } else {
                archon_embodied::ActionProposal::default_resources()
            }
        } else {
            proposal.required_resources.clone()
        };

        if !resources.is_empty() {
            if let Err(e) = self.locks.try_acquire(&resources, "executive") {
                episode.push("lock_denied", serde_json::json!({ "error": e.to_string() }));
                episode.finish();
                watch.abort();
                return Ok((ExecutionResult::rejected(e.to_string()), episode));
            }
        }

        let verdict = safety.check_proposal(&proposal, &state).await;
        episode.push(
            "safety_proposal",
            serde_json::to_value(&verdict).unwrap_or_default(),
        );
        if let SafetyVerdict::Deny { reason } = verdict {
            self.locks.release_all("executive");
            episode.finish();
            watch.abort();
            self.events.publish(RuntimeEvent::SafetyFault {
                reason: reason.clone(),
            });
            return Ok((ExecutionResult::rejected(reason), episode));
        }

        let commands = self.chronos.interpolate_proposal(&proposal);
        episode.push(
            "chronos",
            serde_json::json!({ "commands": commands.len(), "rate_hz": self.chronos.rate_hz }),
        );

        // Per-command safety sample (first, mid, last) for MVP.
        for idx in sample_indices(commands.len()) {
            if let Some(cmd) = commands.get(idx) {
                let v = safety.check_command(cmd, &state).await;
                if let SafetyVerdict::Deny { reason } = v {
                    episode.push(
                        "safety_command_deny",
                        serde_json::json!({ "index": idx, "reason": reason }),
                    );
                    self.locks.release_all("executive");
                    episode.finish();
                    watch.abort();
                    return Ok((ExecutionResult::rejected(reason), episode));
                }
            }
        }

        let result = {
            let mut b = backend.lock().await;
            let stream_result = b
                .execute_stream(&commands, &self.cancel, self.config.control_step_ms)
                .await
                .context("execute stream")?;

            if self.cancel.is_cancelled() {
                let _ = b.estop().await;
                episode.push("estop_or_cancel", serde_json::json!({ "cancelled": true }));
            }

            let _ = b.shutdown().await;
            stream_result
        };

        episode.push(
            "execution_result",
            serde_json::to_value(&result).unwrap_or_default(),
        );

        if matches!(result.status, ExecutionStatus::Completed) {
            self.events.publish(RuntimeEvent::TaskCompleted {
                task_id: self.config.task_id.clone(),
            });
        }

        self.locks.release_all("executive");
        episode.finish();
        watch.abort();
        Ok((result, episode))
    }
}

fn sample_indices(n: usize) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    if n == 1 {
        return vec![0];
    }
    vec![0, n / 2, n - 1]
}
