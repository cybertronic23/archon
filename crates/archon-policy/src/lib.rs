//! Embodied policies and safety gates.

pub mod color_blob;
pub mod mock;
pub mod safety;

pub use color_blob::ColorBlobPolicy;
pub use mock::{MockPolicy, VlaAdapterStub, WamAdapterStub};
pub use safety::LimitSafetyGate;
