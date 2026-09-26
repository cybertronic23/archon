//! Camera sources for perception.

use anyhow::Result;
use async_trait::async_trait;

use crate::frame::RgbFrame;

#[async_trait]
pub trait CameraSource: Send + Sync {
    fn name(&self) -> &str;
    async fn next_frame(&mut self) -> Result<RgbFrame>;
}

/// Synthetic scene: gray background + optional red blob for M1 CI.
pub struct SyntheticColorCamera {
    pub width: u32,
    pub height: u32,
    pub with_red_blob: bool,
    frame_index: u64,
}

impl Default for SyntheticColorCamera {
    fn default() -> Self {
        Self {
            width: 64,
            height: 48,
            with_red_blob: true,
            frame_index: 0,
        }
    }
}

impl SyntheticColorCamera {
    pub fn new(width: u32, height: u32, with_red_blob: bool) -> Self {
        Self {
            width,
            height,
            with_red_blob,
            frame_index: 0,
        }
    }

    pub fn with_blob() -> Self {
        Self::default()
    }

    pub fn blank() -> Self {
        Self {
            with_red_blob: false,
            ..Self::default()
        }
    }
}

#[async_trait]
impl CameraSource for SyntheticColorCamera {
    fn name(&self) -> &str {
        "synthetic"
    }

    async fn next_frame(&mut self) -> Result<RgbFrame> {
        self.frame_index += 1;
        let w = self.width as usize;
        let h = self.height as usize;
        let mut rgb = vec![40u8; w * h * 3]; // dark gray

        if self.with_red_blob {
            // Red square in the lower-center of the image.
            let bw = w / 5;
            let bh = h / 5;
            let x0 = (w - bw) / 2;
            let y0 = (h * 2) / 3;
            for y in y0..(y0 + bh).min(h) {
                for x in x0..(x0 + bw).min(w) {
                    let i = (y * w + x) * 3;
                    rgb[i] = 220;
                    rgb[i + 1] = 20;
                    rgb[i + 2] = 20;
                }
            }
        }

        RgbFrame::new(
            self.width,
            self.height,
            archon_embodied::now_us(),
            rgb,
        )
    }
}
