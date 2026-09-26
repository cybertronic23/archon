//! Enrich proprio observations with modalities + annotations.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use archon_embodied::{
    modality_keys, MediaLayout, MediaRef, ModalitySample, Observation,
};

use crate::camera::CameraSource;
use crate::detector::ColorBlobDetector;
use crate::frame::RgbFrame;

pub struct PerceptionBridge {
    camera: Box<dyn CameraSource>,
    detector: ColorBlobDetector,
    /// Optional directory to persist frames: <dir>/media/images.primary/
    media_root: Option<PathBuf>,
    frame_seq: u64,
    pub last_frame: Option<RgbFrame>,
}

impl PerceptionBridge {
    pub fn new(camera: Box<dyn CameraSource>, detector: ColorBlobDetector) -> Self {
        Self {
            camera,
            detector,
            media_root: None,
            frame_seq: 0,
            last_frame: None,
        }
    }

    pub fn with_media_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.media_root = Some(root.into());
        self
    }

    pub fn camera_name(&self) -> &str {
        self.camera.name()
    }

    /// Merge camera + detections into a body observation from the backend.
    pub async fn enrich(&mut self, mut body: Observation) -> Result<Observation> {
        let frame = self.camera.next_frame().await.context("camera next_frame")?;
        self.frame_seq += 1;

        let relative_uri = format!(
            "media/{}/{:06}.ppm",
            modality_keys::IMAGES_PRIMARY,
            self.frame_seq
        );

        if let Some(root) = &self.media_root {
            let path = root.join(&relative_uri);
            frame
                .write_ppm(&path)
                .with_context(|| format!("write frame {}", path.display()))?;
        }

        let uri_for_obs = if let Some(root) = &self.media_root {
            root.join(&relative_uri).to_string_lossy().into_owned()
        } else {
            relative_uri.clone()
        };

        body.modalities.insert(
            modality_keys::IMAGES_PRIMARY.into(),
            ModalitySample {
                stamp_us: frame.stamp_us,
                frame_id: "camera_link".into(),
                encoding: "rgb8".into(),
                layout: MediaLayout {
                    width: Some(frame.width),
                    height: Some(frame.height),
                    channels: Some(3),
                    count: None,
                },
                storage: MediaRef::uri(uri_for_obs),
            },
        );

        if let Some(ann) = self.detector.detect(&frame) {
            body.annotations.push(ann);
        }

        body.stamp_us = archon_embodied::now_us();
        self.last_frame = Some(frame);
        Ok(body)
    }

    pub fn media_relative_glob(root: &Path) -> PathBuf {
        root.join("media").join(modality_keys::IMAGES_PRIMARY)
    }
}
