//! RGB frame buffer (row-major rgb8).

#[derive(Debug, Clone)]
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub stamp_us: u64,
    /// Length = width * height * 3
    pub rgb: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: u32, height: u32, stamp_us: u64, rgb: Vec<u8>) -> anyhow::Result<Self> {
        let expected = (width as usize) * (height as usize) * 3;
        if rgb.len() != expected {
            anyhow::bail!(
                "rgb len {} != expected {} for {}x{}",
                rgb.len(),
                expected,
                width,
                height
            );
        }
        Ok(Self {
            width,
            height,
            stamp_us,
            rgb,
        })
    }

    /// Write binary PPM (P6) — no extra image crate dependency.
    pub fn write_ppm(&self, path: &std::path::Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = format!("P6\n{} {}\n255\n", self.width, self.height).into_bytes();
        out.extend_from_slice(&self.rgb);
        std::fs::write(path, out)?;
        Ok(())
    }
}
