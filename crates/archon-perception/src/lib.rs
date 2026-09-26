//! Perception bridge: cameras, detectors, Observation enrich (M1).

pub mod bridge;
pub mod camera;
pub mod detector;
pub mod frame;

pub use bridge::PerceptionBridge;
pub use camera::{CameraSource, SyntheticColorCamera};
pub use detector::ColorBlobDetector;
pub use frame::RgbFrame;
