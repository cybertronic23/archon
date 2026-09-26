//! Simple red-blob detector (HSV-ish threshold on RGB).

use archon_embodied::{modality_keys, Annotation};

use crate::frame::RgbFrame;

#[derive(Debug, Clone)]
pub struct ColorBlobDetector {
    pub label: String,
    /// Minimum red channel and max green/blue to count as red.
    pub min_r: u8,
    pub max_gb: u8,
    pub min_pixels: usize,
}

impl Default for ColorBlobDetector {
    fn default() -> Self {
        Self {
            label: "red_blob".into(),
            min_r: 150,
            max_gb: 80,
            min_pixels: 20,
        }
    }
}

impl ColorBlobDetector {
    pub fn detect(&self, frame: &RgbFrame) -> Option<Annotation> {
        let w = frame.width as usize;
        let h = frame.height as usize;
        let mut min_x = w;
        let mut min_y = h;
        let mut max_x = 0usize;
        let mut max_y = 0usize;
        let mut count = 0usize;

        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                let r = frame.rgb[i];
                let g = frame.rgb[i + 1];
                let b = frame.rgb[i + 2];
                if r >= self.min_r && g <= self.max_gb && b <= self.max_gb {
                    count += 1;
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                }
            }
        }

        if count < self.min_pixels {
            return None;
        }

        let bw = (max_x - min_x + 1) as f64;
        let bh = (max_y - min_y + 1) as f64;
        let score = (count as f64 / (w * h) as f64).clamp(0.0, 1.0);
        Some(Annotation::detection(
            frame.stamp_us,
            modality_keys::IMAGES_PRIMARY,
            &self.label,
            score.max(0.5),
            [min_x as f64, min_y as f64, bw, bh],
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{CameraSource, SyntheticColorCamera};

    #[tokio::test]
    async fn detects_red_on_synthetic() {
        let mut cam = SyntheticColorCamera::with_blob();
        let frame = cam.next_frame().await.unwrap();
        let det = ColorBlobDetector::default();
        assert!(det.detect(&frame).is_some());
    }

    #[tokio::test]
    async fn blank_has_no_blob() {
        let mut cam = SyntheticColorCamera::blank();
        let frame = cam.next_frame().await.unwrap();
        let det = ColorBlobDetector::default();
        assert!(det.detect(&frame).is_none());
    }
}
