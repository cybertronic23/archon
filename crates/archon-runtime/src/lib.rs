//! Embodied runtime: single execution authority, events, locks, arbiter.

pub mod arbiter;
pub mod event_bus;
pub mod executive;
pub mod resource_lock;

pub use arbiter::{Arbiter, ArbiterAction};
pub use event_bus::{EventBus, EventPriority, RuntimeEvent};
pub use executive::{Executive, ExecutiveConfig};
pub use resource_lock::ResourceLocks;
