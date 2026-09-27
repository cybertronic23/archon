//! Embodied policies and safety gates.

pub mod chat;
pub mod color_blob;
pub mod instruction;
pub mod llm_policy;
pub mod mock;
pub mod safety;

pub use color_blob::ColorBlobPolicy;
pub use instruction::{InstructionPolicy, MotionPrimitive, RobotKind};
pub use llm_policy::{LlmPolicy, LlmPolicyConfig};
pub use mock::{MockPolicy, VlaAdapterStub, WamAdapterStub};
pub use safety::LimitSafetyGate;
