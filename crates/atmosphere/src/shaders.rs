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

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Optical,
    Directions,
    IncidentFirst,
    IncidentMultiple,
    Moments,
    Source,
    GroundDirect,
    GroundMultiple,
    Sky,
    Aerial,
}

impl Stage {
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// 固定balanced求解的十个生产入口；stage内binding从零连续编号。
/// 首轮与后续散射、初始地面直射与累计地面照明分别持有自己的资源接口。
pub const SHADERS: [ShaderBinary; 10] = [
    binary!("optical_depth", [8, 8, 1]),
    binary!("prepare_directions", [64, 1, 1]),
    binary!("trace_incident_first", [64, 1, 1]),
    binary!("trace_incident_multiple", [64, 1, 1]),
    binary!("incident_moments", [64, 1, 1]),
    binary!("scattering_source", [8, 8, 1]),
    binary!("ground_direct", [64, 1, 1]),
    binary!("ground_multiple", [64, 1, 1]),
    binary!("build_sky", [8, 8, 1]),
    binary!("build_aerial", [4, 4, 4]),
];
