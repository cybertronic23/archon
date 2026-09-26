//! Resource locks for arm / gripper / etc.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{bail, Result};
use archon_embodied::ResourceKind;

#[derive(Debug, Default)]
pub struct ResourceLocks {
    holders: Mutex<HashMap<ResourceKind, String>>,
}

impl ResourceLocks {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn try_acquire(&self, resources: &[ResourceKind], holder: &str) -> Result<()> {
        let mut map = self.holders.lock().expect("resource lock poisoned");
        for r in resources {
            if let Some(existing) = map.get(r) {
                if existing != holder {
                    bail!("resource {r:?} held by '{existing}', cannot acquire for '{holder}'");
                }
            }
        }
        for r in resources {
            map.insert(*r, holder.to_string());
        }
        Ok(())
    }

    pub fn release(&self, resources: &[ResourceKind], holder: &str) {
        let mut map = self.holders.lock().expect("resource lock poisoned");
        for r in resources {
            if map.get(r).map(|h| h.as_str()) == Some(holder) {
                map.remove(r);
            }
        }
    }

    pub fn release_all(&self, holder: &str) {
        let mut map = self.holders.lock().expect("resource lock poisoned");
        map.retain(|_, h| h != holder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_arm() {
        let locks = ResourceLocks::new();
        locks
            .try_acquire(&[ResourceKind::Arm], "task_a")
            .unwrap();
        assert!(locks
            .try_acquire(&[ResourceKind::Arm], "task_b")
            .is_err());
        locks.release(&[ResourceKind::Arm], "task_a");
        locks
            .try_acquire(&[ResourceKind::Arm], "task_b")
            .unwrap();
    }
}
