//! 最近点纹理的精确二值 opacity micromap 准备；不依赖设备、材质或场景策略。
//! 仅接受 atlas 内的 texel 整数顶点，并验证每个微三角内部完全落入同一个 texel。
//! 未对齐返回 None，不能用中心点近似代替准入；边界按硬件 OMM 归属处理。
//! 准入保证微三角内部覆盖恒定，不保证边界射线与软件浮点 UV 插值逐位一致。
//! UV 和 sprite 边界采用同一 atlas；纹理覆盖在该资源世代内必须不可变。
//! Vulkan bird 次序见 <https://docs.vulkan.org/spec/latest/chapters/VK_KHR_opacity_micromap/micromaps.html>。
#![forbid(unsafe_code)]

/// 准备输入或内存申请失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 图像大小、裁剪矩形、UV 数值或细分预算无效。
    #[error("二值 OMM 输入无效")]
    Input,
    /// 保留底层分配失败。
    #[error(transparent)]
    Allocation(#[from] std::collections::TryReserveError),
}

/// 资源准备时收集的实际 quad UV；material 是调用方的稳定身份。
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    /// 完整资源世代中的材质索引。
    pub material: u32,
    /// 与 BLAS 两三角 (012,023) 一致的有序 UV。
    pub uv: [[f32; 2]; 4],
}

/// Vulkan 2-state、LSB 优先、bird 顺序的独占结果。
#[derive(Debug, PartialEq, Eq)]
pub struct Map {
    /// 细分等级，最多 10，每图最多 128 KiB。
    pub level: u16,
    /// 恰好 ceil(4^level/8) 字节，尾位为零。
    pub data: Vec<u8>,
}

#[derive(Clone, Copy, Default)]
struct Task {
    vertices: [[u16; 2]; 3],
    level: u16,
}

/// 不分配、不递归的 bird 顺序微三角遍历，坐标分母为 2^level。
pub struct MicroTriangles {
    stack: [Task; 31],
    len: usize,
    remaining: usize,
}
impl MicroTriangles {
    /// 根三角为 (0,0),(1,0),(0,1)；超过 10 级返回输入错误。
    pub fn new(level: u16) -> Result<Self, Error> {
        if level > 10 {
            return Err(Error::Input);
        }
        let mut value = Self {
            stack: [Task::default(); 31],
            len: 1,
            remaining: 1 << (2 * level),
        };
        let side = 1 << level;
        value.stack[0] = Task {
            vertices: [[0, 0], [side, 0], [0, side]],
            level,
        };
        Ok(value)
    }
}
impl Iterator for MicroTriangles {
    type Item = [[u16; 2]; 3];
    fn next(&mut self) -> Option<Self::Item> {
        while self.len != 0 {
            self.len -= 1;
            let task = self.stack[self.len];
            if task.level == 0 {
                self.remaining -= 1;
                return Some(task.vertices);
            }
            let [a, b, c] = task.vertices;
            let mid = |a: [u16; 2], b: [u16; 2]| [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
            let (ab, ac, bc) = (mid(a, b), mid(a, c), mid(b, c));
            for vertices in [[bc, ac, c], [ab, b, bc], [ac, bc, ab], [a, ab, ac]] {
                self.stack[self.len] = Task {
                    vertices,
                    level: task.level - 1,
                };
                self.len += 1;
            }
        }
        None
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl ExactSizeIterator for MicroTriangles {}
impl std::iter::FusedIterator for MicroTriangles {}

/// 共享只读 RGBA8 atlas；bounds 是最近点寻址时夹取的半开像素矩形。
pub struct Texture<'a> {
    /// 非零宽高，最多 16384。
    pub extent: [u32; 2],
    /// 逐行 RGBA8，恰好 width*height*4 字节。
    pub rgba: &'a [u8],
    /// `[x0,y0,x1,y1]` 非空且在图像内。
    pub bounds: [u32; 4],
    /// alpha 字节大于等于此值时不透明。
    pub cutoff: u8,
}

/// 在设备允许的 max_level 内准备；未精确对齐或超预算返回 None。
/// 不读取材质、世界或 GPU；仅本次调用持有临时输出。输入错误或分配失败返回 Err。
pub fn aligned(
    texture: &Texture<'_>,
    uv: [[f32; 2]; 3],
    max_level: u16,
) -> Result<Option<Map>, Error> {
    let [w, h] = texture.extent;
    let [x0, y0, x1, y1] = texture.bounds;
    if w == 0
        || h == 0
        || w > 16384
        || h > 16384
        || max_level > 10
        || texture.rgba.len() as u64 != u64::from(w) * u64::from(h) * 4
        || x0 >= x1
        || y0 >= y1
        || x1 > w
        || y1 > h
        || uv.iter().flatten().any(|v| !v.is_finite())
    {
        return Err(Error::Input);
    }
    // 场景可合法使用 atlas 外的 clamp UV；未准入时保留原来的软件寻址。
    if uv.iter().flatten().any(|v| !(0.0..=1.0).contains(v)) {
        return Ok(None);
    }
    let mut texels = [[0i64; 2]; 3];
    for i in 0..3 {
        for axis in 0..2 {
            let value = f64::from(uv[i][axis]) * f64::from(texture.extent[axis]);
            if value.fract() != 0.0 {
                return Ok(None);
            }
            texels[i][axis] = value as i64;
        }
    }
    let span = (0..2)
        .map(|axis| {
            let values = texels.map(|p| p[axis]);
            (values.into_iter().max().unwrap() - values.into_iter().min().unwrap()) as u32
        })
        .max()
        .unwrap();
    let level = span.max(1).next_power_of_two().ilog2() as u16;
    if level > max_level {
        return Ok(None);
    }
    let denominator = 1i64 << level;
    let mut data = Vec::new();
    let bytes = (1usize << (2 * level)).div_ceil(8);
    data.try_reserve_exact(bytes)?;
    data.resize(bytes, 0);
    for (index, triangle) in MicroTriangles::new(level)?.enumerate() {
        let mut cell = [0; 2];
        for axis in 0..2 {
            let values = triangle.map(|p| {
                texels[0][axis] * denominator
                    + (texels[1][axis] - texels[0][axis]) * i64::from(p[0])
                    + (texels[2][axis] - texels[0][axis]) * i64::from(p[1])
            });
            let low = values.into_iter().min().unwrap();
            let high = values.into_iter().max().unwrap();
            let clamp = |v: i64| {
                v.clamp(
                    i64::from(texture.bounds[axis]),
                    i64::from(texture.bounds[axis + 2]) - 1,
                )
            };
            let first = clamp(low.div_euclid(denominator));
            let last = clamp((if high == low { high } else { high - 1 }).div_euclid(denominator));
            if first != last {
                return Ok(None);
            }
            cell[axis] = first as usize;
        }
        if texture.rgba[(cell[1] * w as usize + cell[0]) * 4 + 3] >= texture.cutoff {
            data[index / 8] |= 1 << (index % 8);
        }
    }
    Ok(Some(Map { level, data }))
}

#[cfg(test)]
mod tests;
