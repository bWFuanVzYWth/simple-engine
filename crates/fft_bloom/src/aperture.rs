//! 每种光瞳使用一张标准 RGB32F KTX2。中心副本在构建时从同一张图裁出。
use super::{APERTURE_SIDE, Aperture, Error};

impl Aperture {
    /// 借用完整的二维线性 RGB32F KTX2 文件，唯一 mip 层使用 Zstd 无损压缩。
    pub const fn ktx2(self) -> &'static [u8] {
        match self {
            Self::Camera => include_bytes!("../data/camera.ktx2"),
            Self::Pupil => include_bytes!("../data/pupil.ktx2"),
        }
    }

    /// 展开完整的 1825×1825 标定，返回从左上开始逐行交错的小端 RGB f32。
    /// # Errors
    /// 格式、尺寸、分配或解压错误逐层返回。
    pub fn rgb32le(self) -> Result<Vec<u8>, Error> {
        Ok(resource_pack::texture::read_rgb32f(
            self.ktx2(),
            [APERTURE_SIDE as u32; 2],
        )?)
    }

    #[cfg(all(feature = "vulkan", not(target_arch = "wasm32")))]
    pub(super) const fn core_rgb32le(
        self,
    ) -> &'static [u8; super::aperture_layout::CORE_SIDE.pow(2) * 12] {
        match self {
            Self::Camera => include_bytes!(concat!(env!("OUT_DIR"), "/camera-core.rgb32le")),
            Self::Pupil => include_bytes!(concat!(env!("OUT_DIR"), "/pupil-core.rgb32le")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn shipped_calibrations_match_original_exports() -> Result<(), Error> {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../data/presets.json"))
                .expect("随包标定清单是 JSON");
        for (shape, name) in [(Aperture::Camera, "camera"), (Aperture::Pupil, "pupil")] {
            let hash = manifest[name]["sha256"]
                .as_str()
                .expect("每项标定含原始 SHA-256");
            assert_eq!(
                Sha256::digest(shape.rgb32le()?)
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect::<String>(),
                hash
            );
        }
        Ok(())
    }
}
