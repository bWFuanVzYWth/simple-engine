/// 一个受控 Slang 计算入口；顺序与 [`SHADERS`] 相同。
pub struct ShaderBinary {
    /// 原始 SPIR-V 字节。
    pub spirv: &'static [u8],
    /// 保留在 SPIR-V 中的入口名。
    pub entry: &'static str,
    /// Slang 声明的工作组尺寸。
    pub workgroup: [u32; 3],
}

macro_rules! binary {
    ($entry:literal, $size:expr) => {
        ShaderBinary {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/", $entry, ".spv")),
            entry: $entry,
            workgroup: $size,
        }
    };
}

/// 八个生产入口；stage 内 binding 从零连续编号。
pub const SHADERS: [ShaderBinary; 8] = [
    binary!("optical_depth", [8, 8, 1]),
    binary!("prepare_directions", [64, 1, 1]),
    binary!("trace_incident", [64, 1, 1]),
    binary!("incident_moments", [64, 1, 1]),
    binary!("scattering_source", [8, 8, 1]),
    binary!("ground_irradiance", [64, 1, 1]),
    binary!("build_sky", [8, 8, 1]),
    binary!("build_aerial", [4, 4, 4]),
];
