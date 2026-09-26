//! Arbiter: maps events to cancel / estop actions.

use archon_embodied::CancelToken;

use crate::event_bus::{EventPriority, RuntimeEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbiterAction {
    Continue,
    CancelTask,
    EStop,
}

/// Deterministic priority arbiter — not an LLM.
#[derive(Debug, Default)]
pub struct Arbiter {
    highest_seen: Option<EventPriority>,
}

impl Arbiter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn decide(&mut self, event: &RuntimeEvent) -> ArbiterAction {
        let p = event.priority();
        if self.highest_seen.map(|h| p > h).unwrap_or(true) {
            self.highest_seen = Some(p);
        }
        match event {
            RuntimeEvent::EStop { .. } => ArbiterAction::EStop,
            RuntimeEvent::UserStop { .. } | RuntimeEvent::SafetyFault { .. } => {
                ArbiterAction::CancelTask
            }
            _ => ArbiterAction::Continue,
        }
    }

    pub fn apply(&mut self, event: &RuntimeEvent, cancel: &CancelToken) -> ArbiterAction {
        let action = self.decide(event);
        if matches!(action, ArbiterAction::CancelTask | ArbiterAction::EStop) {
            cancel.cancel();
        }
        action
    }

    pub fn reset(&mut self) {
        self.highest_seen = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estop_beats_user_stop() {
        let mut a = Arbiter::new();
        let cancel = CancelToken::new();
        assert_eq!(
            a.apply(
                &RuntimeEvent::UserStop {
                    reason: "stop".into()
                },
                &cancel
            ),
            ArbiterAction::CancelTask
        );
        assert!(cancel.is_cancelled());
        assert_eq!(
            a.decide(&RuntimeEvent::EStop {
                reason: "hit".into()
            }),
            ArbiterAction::EStop
        );
    }
}
