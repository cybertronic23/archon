//! NDJSON bridge protocol v1 (Rust ↔ Python sim workers).

use archon_embodied::{
    modality_keys, MediaLayout, MediaRef, MediaStorageKind, ModalitySample, Observation,
    ProprioState, JointState,
};
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Hello {
        protocol_version: u32,
        platform: String,
        dof: usize,
        joint_names: Vec<String>,
        render: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        media_root: Option<String>,
        /// Absolute path to MJCF / scene XML.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        camera: Option<String>,
        /// If true (default), worker discovers hinge joints from the model when names empty.
        #[serde(default = "default_true")]
        auto_joints: bool,
        /// Open MuJoCo interactive viewer window (requires local display).
        #[serde(default)]
        viewer: bool,
        /// Directory to dump video frames (PPM).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        record_dir: Option<String>,
        /// Final mp4 path (worker runs ffmpeg after motion if available).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        video_out: Option<String>,
        #[serde(default)]
        record_width: Option<u32>,
        #[serde(default)]
        record_height: Option<u32>,
    },
    Reset,
    /// Return current observation without advancing physics.
    Observe,
    Command {
        stamp_us: u64,
        names: Vec<String>,
        positions: Vec<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        gripper_open: Option<f64>,
    },
    Estop {
        reason: String,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    HelloOk {
        protocol_version: u32,
        platform: String,
        #[serde(default)]
        joint_names: Vec<String>,
        #[serde(default)]
        dof: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_path: Option<String>,
    },
    Observation {
        stamp_us: u64,
        proprio: ProprioWire,
        #[serde(default)]
        modalities: Vec<ModalityWire>,
    },
    Ack,
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProprioWire {
    pub joint_names: Vec<String>,
    pub positions: Vec<f64>,
    #[serde(default)]
    pub gripper_open: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModalityWire {
    pub key: String,
    pub stamp_us: u64,
    pub frame_id: String,
    pub encoding: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    pub uri: String,
}

impl ServerMsg {
    pub fn into_observation(self) -> anyhow::Result<Observation> {
        match self {
            ServerMsg::Observation {
                stamp_us,
                proprio,
                modalities,
            } => {
                let mut obs = Observation::from_proprio(ProprioState::new(
                    JointState::new(proprio.joint_names, proprio.positions),
                    proprio.gripper_open,
                ));
                obs.stamp_us = stamp_us;
                for m in modalities {
                    let key = if m.key.is_empty() {
                        modality_keys::IMAGES_PRIMARY.to_string()
                    } else {
                        m.key
                    };
                    obs.modalities.insert(
                        key,
                        ModalitySample {
                            stamp_us: m.stamp_us,
                            frame_id: m.frame_id,
                            encoding: m.encoding,
                            layout: MediaLayout {
                                width: m.width,
                                height: m.height,
                                channels: Some(3),
                                count: None,
                            },
                            storage: MediaRef {
                                kind: MediaStorageKind::InlineUri,
                                uri: Some(m.uri),
                                topic: None,
                                byte_size: None,
                            },
                        },
                    );
                }
                Ok(obs)
            }
            ServerMsg::Error { message } => anyhow::bail!("worker error: {message}"),
            other => anyhow::bail!("expected observation, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip() {
        let msg = ClientMsg::Hello {
            protocol_version: PROTOCOL_VERSION,
            platform: "mujoco".into(),
            dof: 6,
            joint_names: vec!["j1".into()],
            render: true,
            media_root: Some("/tmp/x".into()),
            model_path: Some("/tmp/model.xml".into()),
            camera: Some("scene".into()),
            auto_joints: true,
            viewer: false,
            record_dir: None,
            video_out: None,
            record_width: None,
            record_height: None,
        };
        let s = serde_json::to_string(&msg).unwrap();
        let back: ClientMsg = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, ClientMsg::Hello { .. }));
    }

    #[test]
    fn observation_to_archon() {
        let msg = ServerMsg::Observation {
            stamp_us: 42,
            proprio: ProprioWire {
                joint_names: vec!["joint_1".into()],
                positions: vec![0.1],
                gripper_open: 0.5,
            },
            modalities: vec![ModalityWire {
                key: modality_keys::IMAGES_PRIMARY.into(),
                stamp_us: 42,
                frame_id: "camera_link".into(),
                encoding: "rgb8".into(),
                width: Some(64),
                height: Some(48),
                uri: "media/images.primary/000001.ppm".into(),
            }],
        };
        let obs = msg.into_observation().unwrap();
        assert_eq!(obs.stamp_us, 42);
        assert!(obs.modalities.contains_key(modality_keys::IMAGES_PRIMARY));
    }
}
