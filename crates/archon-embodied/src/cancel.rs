use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared cancellation token for preempting in-flight motion.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    inner: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.inner.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.inner.store(false, Ordering::SeqCst);
    }

    /// Child token that shares the same flag (cancel either side cancels both).
    pub fn clone_shared(&self) -> Self {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_propagates() {
        let t = CancelToken::new();
        let t2 = t.clone_shared();
        assert!(!t.is_cancelled());
        t2.cancel();
        assert!(t.is_cancelled());
    }
}
