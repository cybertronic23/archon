//! Event bus for embodied runtime (stop, estop, faults).

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Priority-ordered runtime events. Higher priority wins arbitration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventPriority {
    ActiveTask = 0,
    UserStop = 1,
    SafetyFault = 2,
    EStop = 3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    UserStop { reason: String },
    EStop { reason: String },
    SafetyFault { reason: String },
    TaskCompleted { task_id: String },
    Custom { name: String, detail: String },
}

impl RuntimeEvent {
    pub fn priority(&self) -> EventPriority {
        match self {
            RuntimeEvent::EStop { .. } => EventPriority::EStop,
            RuntimeEvent::SafetyFault { .. } => EventPriority::SafetyFault,
            RuntimeEvent::UserStop { .. } => EventPriority::UserStop,
            RuntimeEvent::TaskCompleted { .. } | RuntimeEvent::Custom { .. } => {
                EventPriority::ActiveTask
            }
        }
    }

    pub fn should_preempt(&self) -> bool {
        matches!(
            self,
            RuntimeEvent::UserStop { .. }
                | RuntimeEvent::EStop { .. }
                | RuntimeEvent::SafetyFault { .. }
        )
    }
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<RuntimeEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity.max(16));
        Self { tx }
    }

    pub fn publish(&self, event: RuntimeEvent) {
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.tx.subscribe()
    }
}
