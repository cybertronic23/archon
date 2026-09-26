//! Embodied Agent OS — core types and traits.
//!
//! This crate is the physical-plane counterpart to `archon-core`.
//! It does **not** depend on LLM tool_use; policies emit `ActionProposal`s
//! that pass through deterministic safety before reaching a `RobotBackend`.

pub mod cancel;
pub mod traits;
pub mod types;

pub use cancel::CancelToken;
pub use traits::{Policy, RobotBackend, RobotBackendExt, SafetyGate};
pub use types::*;
