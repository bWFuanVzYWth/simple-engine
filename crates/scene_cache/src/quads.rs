//! 地形 quad 的无损位置流。GPU 以局部 primitive 的高位查询 quad，低位选择 012 或 023。
use crate::{Error, Identities};
use std::collections::HashMap;

const OFFSET: u32 = (1 << 29) - 1;
const ESCAPE: u32 = 1 << 31;

/// 紧凑矩形的表面目录：形状的轴、起角、格距与完整的 48B 应用属性同属一条共享记录。
/// 面头提供 32 位表面字。模板的前三个附加 word 可保存回退面的源格，整条记录为 64B。
pub type QuadSurface = [u32; 16];

/// 整数网格矩形的有向投影表面。起角为局部平面上两个切轴的最小值，绕序由法线决定。
/// UV 和原点均沿有向轴投影到网格单位，宽高写入属性标记的低 16 位。
pub struct ProjectedRectangle {
    /// 所属体素，按当前 LOD 的格距计。
    pub anchor: [i32; 3],
    /// 沿法线轴的后续两轴的宽、高。
    pub extent: [u32; 2],
    /// ±1/2/3 分别表示朝向 ±X/Y/Z。
    pub normal: i8,
    /// 一个格距为 2^lod 个位置单位。
    pub lod: u8,
    /// 两个 UV 坐标使用的有向轴，编码与 normal 相同。
    pub uv_axes: [i8; 2],
    /// 应用材料字与高 16 位表面标记。
    pub material_flags: [u32; 2],
}

fn projection_bits(axes: [i8; 2]) -> u32 {
    let code = |a: i8| u32::from(a.unsigned_abs() - 1) | (u32::from(a < 0) << 2);
    1 << 9 | code(axes[0]) << 10 | code(axes[1]) << 13
}

/// 只在一份区域包的构造期间持有模板查找表；驻留记录不保留反查哈希表。
#[derive(Default)]
pub struct QuadEncoder {
    stream: PackedQuads,
    lookup: HashMap<QuadSurface, u32>,
}

fn rectangle(points: [[f32; 3]; 4], lod: u8) -> Option<(u32, u32)> {
    let scale = (1u32.checked_shl(u32::from(lod))?) as f32;
    let grid = points.map(|p| p.map(|v| (v / scale) as u8));
    if (0..4).any(|i| {
        (0..3).any(|j| {
            grid[i][j] > 64 || (f32::from(grid[i][j]) * scale).to_bits() != points[i][j].to_bits()
        })
    }) {
        return None;
    }
    let [a, b, c, d] = grid;
    let axis = (0..3).find(|&i| a[i] == c[i])?;
    let u = (axis + 1) % 3;
    let v = (axis + 2) % 3;
    let width = a[u].abs_diff(c[u]);
    let height = a[v].abs_diff(c[v]);
    if width == 0 || height == 0 || a[axis] != b[axis] || a[axis] != d[axis] {
        return None;
    }
    let along_u = b[u] == c[u] && b[v] == a[v] && d[u] == a[u] && d[v] == c[v];
    let along_v = b[v] == c[v] && b[u] == a[u] && d[v] == a[v] && d[u] == c[u];
    if !along_u && !along_v {
        return None;
    }
    let flip_u = a[u] > c[u];
    let flip_v = a[v] > c[v];
    let positive = along_u ^ flip_u ^ flip_v;
    let mut anchor: [u8; 3] = std::array::from_fn(|i| a[i].min(c[i]));
    anchor[axis] = anchor[axis].checked_sub(u8::from(positive))?;
    if anchor[axis] >= 64 {
        return None;
    }
    let [x, y, z] = anchor.map(u32::from);
    let packed = x
        | y << 6
        | z << 12
        | u32::from(width - 1) << 18
        | u32::from(height - 1) << 24
        | u32::from(positive) << 30;
    let layout =
        axis as u32 | u32::from(flip_u) << 2 | u32::from(flip_v) << 3 | u32::from(lod) << 4;
    Some((packed, layout))
}
const WORDS: [usize; 4] = [2, 3, 6, 12];

// 一格内的十二个角坐标只需十二位。适用于对角草卡及其他由单位格角组成的模型面。
fn cell_quad(points: [[f32; 3]; 4], lod: u8, cell: Option<[i32; 3]>) -> Option<u32> {
    let scale = (1u32.checked_shl(u32::from(lod))?) as f32;
    let anchor: [u32; 3] = if let Some(c) = cell {
        if c.iter()
            .any(|&v| v < 0 || (v as u32) >> lod >= 64 || ((v as u32) >> lod) << lod != v as u32)
        {
            return None;
        }
        c.map(|v| v as u32 >> lod)
    } else {
        std::array::from_fn(|j| {
            (points.iter().map(|p| p[j]).fold(f32::INFINITY, f32::min) / scale).clamp(0., 63.)
                as u32
        })
    };
    let mut mask = 0;
    for (i, p) in points.iter().enumerate() {
        for j in 0..3 {
            if (anchor[j] as f32 * scale).to_bits() == p[j].to_bits() {
                continue;
            }
            if ((anchor[j] + 1) as f32 * scale).to_bits() != p[j].to_bits() {
                return None;
            }
            mask |= 1u32 << (i * 3 + j);
        }
    }
    Some(ESCAPE | mask << 18 | anchor[0] | anchor[1] << 6 | anchor[2] << 12)
}
/// 地形 PCIe 包：每面两个 word，分别为几何与表面索引。同一模板的轴由所有矩形共享。
/// 64³ 格内的矩形占 8B，三轴锚点各 6 bit，两边长减一各 6 bit，正法线占 1 bit。
/// 单格模型同样占 8B，四个角各用 3 bit。其余面通过偏移读取无损净荷，保留绕序、退化半面及 FP32 位值。
#[derive(Default, Debug)]
pub struct PackedQuads {
    headers: Vec<[u32; 2]>,
    surfaces: Vec<QuadSurface>,
    payload: Vec<u32>,
    inherited: Vec<Inherited>,
}

#[derive(Debug)]
struct Inherited {
    serial: u64,
    prior: usize,
    next: usize,
    count: usize,
}

impl QuadEncoder {
    /// 预留目录和常见整数矩形的净荷。超出格式寻址范围或分配失败返回错误。
    pub fn with_capacity(count: usize) -> Result<Self, Error> {
        if count > OFFSET as usize / 2 {
            return Err(Error::Capacity);
        }
        let mut stream = Self::default();
        stream.stream.headers.try_reserve_exact(count)?;
        Ok(stream)
    }

    /// 生产输入带有非有限坐标、word 偏移耗尽或申请失败时，保留原流。
    pub fn push(
        &mut self,
        points: [[f32; 3]; 4],
        lod: u8,
        attributes: [[u32; 4]; 3],
    ) -> Result<(), Error> {
        self.insert(points, lod, attributes, None)
    }
    /// 直接接收矩形分解的整数结果。返回 false 表示该矩形超出 64³ 局部格式，可改传完整面。
    /// 域内矩形不生成浮点顶点；无效朝向、投影或分配失败返回错误。
    pub fn push_rectangle(&mut self, rect: &ProjectedRectangle) -> Result<bool, Error> {
        let axis = rect.normal.unsigned_abs();
        if !(1..=3).contains(&axis)
            || rect
                .uv_axes
                .iter()
                .any(|a| !(1..=3).contains(&a.unsigned_abs()))
        {
            return Err(Error::Position);
        }
        let axis = usize::from(axis - 1);
        if rect.lod >= 32
            || rect
                .anchor
                .iter()
                .any(|&v| !(0..64).contains(&v) || (i64::from(v) << rect.lod) > i64::from(i32::MAX))
        {
            return Ok(false);
        }
        let [x, y, z] = rect.anchor.map(|v| v as u32);
        let [w, h] = rect.extent;
        if w == 0
            || h == 0
            || w > 64 - rect.anchor[(axis + 1) % 3] as u32
            || h > 64 - rect.anchor[(axis + 2) % 3] as u32
        {
            return Ok(false);
        }
        let geometry =
            x | y << 6 | z << 12 | (w - 1) << 18 | (h - 1) << 24 | u32::from(rect.normal > 0) << 30;
        let mut surface = [0; 16];
        surface[0] = axis as u32 | u32::from(rect.lod) << 4 | projection_bits(rect.uv_axes);
        surface[12] = rect.material_flags[0];
        surface[13] = rect.material_flags[1] & !0xffff;
        self.append(surface, false, |_| Ok(geometry))?;
        Ok(true)
    }

    /// 写入具有体素源格和有向投影 UV 的面。两个投影轴用 ±1/2/3 表示 ±X/Y/Z。
    /// 属性前八个 word 为四角 UV，最后四个为表面、标记、原点 UV。
    /// 仅在能逐位恢复所有属性与源格时省去模板内的 UV，否则保留实际属性。
    pub fn push_projected(
        &mut self,
        points: [[f32; 3]; 4],
        lod: u8,
        attributes: [[u32; 4]; 3],
        cell: [i32; 3],
        uv_axes: [i8; 2],
    ) -> Result<(), Error> {
        if uv_axes.iter().any(|v| !(1..=3).contains(&v.unsigned_abs())) {
            return Err(Error::Position);
        }
        self.insert(points, lod, attributes, Some((cell, uv_axes)))
    }
    fn insert(
        &mut self,
        points: [[f32; 3]; 4],
        lod: u8,
        attributes: [[u32; 4]; 3],
        projected: Option<([i32; 3], [i8; 2])>,
    ) -> Result<(), Error> {
        let mut rect = rectangle(points, lod);
        if let Some((cell, _)) = projected {
            rect = rect
                .filter(|&(g, _)| (0..3).all(|i| cell[i] == (((g >> (6 * i)) & 63) << lod) as i32));
        }
        let instance = if rect.is_none() {
            cell_quad(points, lod, projected.map(|v| v.0))
        } else {
            None
        };
        let mut layout = rect.map_or(u32::from(lod.min(31)) << 4, |(_, layout)| layout);
        let mut surface = [0; 16];
        surface[4..].copy_from_slice(attributes.as_flattened());
        if let Some((cell, axes)) = projected {
            if rect.is_some() || instance.is_some() {
                let scale = (1u32 << lod) as f32;
                let project = |p: [f32; 3], a: i8| {
                    let v = p[usize::from(a.unsigned_abs() - 1)] / scale;
                    if a < 0 { -v } else { v }
                };
                let uv_match =
                    (0..8).all(|i| project(points[i / 2], axes[i % 2]).to_bits() == surface[4 + i]);
                let origin_match = (0..2)
                    .all(|i| project(cell.map(|v| v as f32), axes[i]).to_bits() == surface[14 + i]);
                let sizes = rect.map_or(257, |(g, _)| {
                    (((g >> 18) & 63) + 1) | ((((g >> 24) & 63) + 1) << 8)
                });
                if uv_match && origin_match && surface[13] & 0xffff == sizes {
                    layout |= projection_bits(axes);
                    surface[4..12].fill(0);
                    surface[13] &= !0xffff;
                    surface[14..16].fill(0);
                }
            } else {
                surface[1..4].copy_from_slice(&cell.map(|v| v as u32));
            }
        }
        surface[0] = layout;
        self.append(surface, instance.is_some(), |stream| {
            if let Some((word, _)) = rect {
                Ok(word)
            } else if let Some(word) = instance {
                Ok(word)
            } else {
                stream.fallback(points)
            }
        })
    }
    fn append(
        &mut self,
        surface: QuadSurface,
        anchored: bool,
        geometry: impl FnOnce(&mut PackedQuads) -> Result<u32, Error>,
    ) -> Result<(), Error> {
        let existing = self.lookup.get(&surface).copied();
        let index = if let Some(id) = existing {
            id
        } else {
            self.lookup.try_reserve(1)?;
            self.stream.surfaces.try_reserve(1)?;
            u32::try_from(self.stream.surfaces.len())
                .ok()
                .filter(|i| *i < ESCAPE)
                .ok_or(Error::Capacity)?
        };
        self.stream.headers.try_reserve(1)?;
        let geometry = geometry(&mut self.stream)?;
        if existing.is_none() {
            self.stream.surfaces.push(surface);
            self.lookup.insert(surface, index);
        }
        self.stream
            .headers
            .push([geometry, index | if anchored { ESCAPE } else { 0 }]);
        Ok(())
    }
    /// 拼入已编码区域或分片的源序范围。只重定位表面索引和回退净荷，完整保留几何字与着色属性。
    /// 申请失败时本次构造应整体丢弃，已发布的源保持有效。
    pub fn extend_packed(
        &mut self,
        source: &PackedQuads,
        range: std::ops::Range<usize>,
    ) -> Result<(), Error> {
        let headers = source.headers.get(range.clone()).ok_or(Error::Capacity)?;
        let base = self.stream.headers.len();
        self.stream.inherited.try_reserve(source.inherited.len())?;
        for s in &source.inherited {
            let first = range.start.max(s.next);
            let end = range.end.min(s.next + s.count);
            if first < end {
                self.stream.inherited.push(Inherited {
                    serial: s.serial,
                    prior: s.prior + (first - s.next),
                    next: base + (first - range.start),
                    count: end - first,
                });
            }
        }
        self.stream.headers.try_reserve(headers.len())?;
        let mut last = None;
        for &[mut geometry, s] in headers {
            let surface = s & !ESCAPE;
            let id = if let Some((_, mapped)) = last.filter(|(prev, _)| *prev == surface) {
                mapped
            } else {
                let row = source.surfaces[surface as usize];
                let id = if let Some(&id) = self.lookup.get(&row) {
                    id
                } else {
                    self.lookup.try_reserve(1)?;
                    self.stream.surfaces.try_reserve(1)?;
                    let id = u32::try_from(self.stream.surfaces.len())
                        .ok()
                        .filter(|v| *v < ESCAPE)
                        .ok_or(Error::Capacity)?;
                    self.stream.surfaces.push(row);
                    self.lookup.insert(row, id);
                    id
                };
                last = Some((surface, id));
                id
            };
            if s & ESCAPE == 0 && geometry & ESCAPE != 0 {
                let at = (geometry & OFFSET) as usize;
                let n = WORDS[((geometry >> 29) & 3) as usize];
                let offset = self.stream.payload.len();
                if offset > OFFSET as usize {
                    return Err(Error::Capacity);
                }
                self.stream.payload.try_reserve(n)?;
                self.stream
                    .payload
                    .extend_from_slice(&source.payload[at..at + n]);
                geometry = (geometry & !OFFSET) | offset as u32;
            }
            self.stream.headers.push([geometry, id | (s & ESCAPE)]);
        }
        Ok(())
    }

    /// 拼入同一驻留区域的不可变范围。匹配阶段凭来源身份直接继承这些面的 ID。
    pub fn extend_resident(
        &mut self,
        source: &ResidentQuads,
        range: std::ops::Range<usize>,
    ) -> Result<(), Error> {
        self.stream.inherited.try_reserve(1)?;
        let record = Inherited {
            serial: source.serial,
            prior: range.start,
            next: self.stream.headers.len(),
            count: range.len(),
        };
        self.extend_packed(source.positions(), range)?;
        self.stream.inherited.push(record);
        Ok(())
    }

    /// 使用驻留面的既有几何和源序，替换显式着色属性。用于只改变材质覆盖的重建。
    pub fn rebind(
        &mut self,
        source: &ResidentQuads,
        i: usize,
        attr: [[u32; 4]; 3],
    ) -> Result<(), Error> {
        let packed = source.positions();
        let &[g, s] = packed.headers.get(i).ok_or(Error::Capacity)?;
        let mut surface = packed.surfaces[(s & !ESCAPE) as usize];
        surface[0] &= 0x1ff;
        surface[4..].copy_from_slice(attr.as_flattened());
        let next = self.stream.headers.len();
        self.stream.inherited.try_reserve(1)?;
        self.append(surface, s & ESCAPE != 0, |dest| {
            if g & ESCAPE == 0 || s & ESCAPE != 0 {
                return Ok(g);
            }
            let at = (g & OFFSET) as usize;
            let n = WORDS[((g >> 29) & 3) as usize];
            let offset = dest.payload.len();
            if offset > OFFSET as usize {
                return Err(Error::Capacity);
            }
            dest.payload.try_reserve(n)?;
            dest.payload.extend_from_slice(&packed.payload[at..at + n]);
            Ok((g & !OFFSET) | offset as u32)
        })?;
        if let Some(span) = self.stream.inherited.last_mut()
            && span.serial == source.serial
            && span.prior + span.count == i
            && span.next + span.count == next
        {
            span.count += 1;
        } else {
            self.stream.inherited.push(Inherited {
                serial: source.serial,
                prior: i,
                next,
                count: 1,
            });
        }
        Ok(())
    }

    /// 结束构造并释放表面反查表，将不可变包移交给驻留记录。
    pub fn finish(self) -> PackedQuads {
        self.stream
    }
}
impl PackedQuads {
    fn fallback(&mut self, points: [[f32; 3]; 4]) -> Result<u32, Error> {
        let octets = points.map(|p| p.map(|v| v as u8));
        let mut words = [0; 12];
        let format = if (0..4)
            .all(|i| (0..3).all(|j| points[i][j].to_bits() == f32::from(octets[i][j]).to_bits()))
        {
            let [a, b, c, d] = octets;
            if (0..3).all(|j| (b[j] == a[j] || b[j] == c[j]) && (d[j] == a[j] || d[j] == c[j])) {
                let mask = |p: [u8; 3]| (0..3).fold(0, |m, j| m | (u8::from(p[j] != a[j]) << j));
                words[0] = u32::from_le_bytes([a[0], a[1], a[2], mask(b)]);
                words[1] = u32::from_le_bytes([c[0], c[1], c[2], mask(d)]);
                0
            } else {
                for (dst, bytes) in words
                    .iter_mut()
                    .zip(octets.as_flattened().as_chunks::<4>().0)
                {
                    *dst = u32::from_le_bytes(*bytes);
                }
                1
            }
        } else {
            let fixed = points.map(|p| p.map(|v| (v * 16.0) as i16));
            if (0..4).all(|i| {
                (0..3)
                    .all(|j| points[i][j].to_bits() == (f32::from(fixed[i][j]) * 0.0625).to_bits())
            }) {
                for (dst, &[a, b]) in words
                    .iter_mut()
                    .zip(fixed.as_flattened().as_chunks::<2>().0)
                {
                    *dst = u32::from(a as u16) | (u32::from(b as u16) << 16);
                }
                2
            } else {
                if points.as_flattened().iter().any(|v| !v.is_finite()) {
                    return Err(Error::Position);
                }
                words.copy_from_slice(points.map(|p| p.map(f32::to_bits)).as_flattened());
                3
            }
        };
        let offset = self.payload.len();
        let len = WORDS[format];
        if offset + len > OFFSET as usize {
            return Err(Error::Capacity);
        }
        self.payload.try_reserve(len)?;
        self.payload.extend_from_slice(&words[..len]);
        Ok(ESCAPE | (format as u32) << 29 | offset as u32)
    }

    /// 每个 quad 一个目录项。GPU 按生产者的源序展开，BLAS 的局部 primitive 编号除以二即源 quad 序号。
    pub const fn headers(&self) -> &[[u32; 2]] {
        self.headers.as_slice()
    }
    /// 与面记录同批提交的共享表面目录。
    pub const fn surfaces(&self) -> &[QuadSurface] {
        self.surfaces.as_slice()
    }
    /// 与目录同批提交的连续净荷，单位为 32 位 word。
    pub const fn payload(&self) -> &[u32] {
        self.payload.as_slice()
    }
    /// 实际传输的目录及坐标净荷字节数。
    pub const fn bytes(&self) -> usize {
        self.headers.len() * 8 + self.payload.len() * 4 + self.surfaces.len() * 64
    }
    /// 两个数组占有的堆容量。
    pub const fn retained_bytes(&self) -> usize {
        self.headers.capacity() * 8 + self.payload.capacity() * 4 + self.surfaces.capacity() * 64
    }

    fn record(&self, index: usize) -> (u32, u32, &[u32]) {
        let [geometry, surface] = self.headers[index];
        if surface & ESCAPE != 0 {
            return (
                geometry,
                self.surfaces[(surface & !ESCAPE) as usize][0] & 0x1ff,
                &[],
            );
        }
        if geometry & ESCAPE == 0 {
            return (geometry, self.surfaces[surface as usize][0] & 0x1ff, &[]);
        }
        let start = (geometry & OFFSET) as usize;
        let format = ((geometry >> 29) & 3) as usize;
        (
            ESCAPE,
            format as u32,
            &self.payload[start..start + WORDS[format]],
        )
    }
    fn surface(&self, index: usize) -> &QuadSurface {
        &self.surfaces[(self.headers[index][1] & !ESCAPE) as usize]
    }
}

/// 同一驻留源内逐 quad 的精确几何记录与非零身份。完全重合且同序的几何按重复出现次序区分。
/// 物理槽、排序及 BLAS 的迁移不参与比较。卸载后释放记录，重新加载从目录事务取得新身份。
#[derive(Default, Debug)]
pub struct ResidentQuads {
    positions: PackedQuads,
    tokens: Vec<u64>,
    first: u64,
    serial: u64,
}
impl ResidentQuads {
    /// 在同一来源的上一份已发布记录中匹配完整四角；哈希仅用于查找，键仍逐 word 比较。
    /// 新身份属于传入的未发布事务，空间耗尽或分配失败由调用者放弃整个事务。
    pub fn prepare(
        positions: PackedQuads,
        old: Option<&Self>,
        ids: &mut Identities,
    ) -> Result<Self, Error> {
        let range = ids.reserve_quads(positions.headers.len())?;
        Self::prepare_reserved(positions, old, range)
    }
    /// 在调度层预留的独占区间内完成身份配对。不同区域可以同时执行，不读取其他任务的分配器。
    pub fn prepare_reserved(
        mut positions: PackedQuads,
        old: Option<&Self>,
        mut ids: crate::QuadIds,
    ) -> Result<Self, Error> {
        let count = positions.headers.len();
        if (ids.0.end - ids.0.start) != count as u64 {
            return Err(Error::Capacity);
        }
        let mut first = ids.0.start;
        let mut tokens = Vec::new();
        let inherited = std::mem::take(&mut positions.inherited);
        if let Some(old) = old {
            let previous = old.positions.headers.len();
            let mut used = Vec::new();
            used.try_reserve_exact(previous)?;
            used.resize(previous, false);
            tokens.try_reserve_exact(count)?;
            tokens.resize(count, 0);
            for span in inherited.iter().filter(|s| s.serial == old.serial) {
                for j in 0..span.count {
                    let prior = span.prior + j;
                    if !used[prior] {
                        tokens[span.next + j] = old.token_at(prior);
                        used[prior] = true;
                    }
                }
            }
            // 源序未移动的面先直接配对。只为剩下发生移动的面建立临时哈希表。
            for i in 0..count.min(previous) {
                if tokens[i] == 0
                    && !used[i]
                    && positions.record(i) == old.positions.record(i)
                    && positions.surface(i) == old.positions.surface(i)
                {
                    tokens[i] = old.token_at(i);
                    used[i] = true;
                }
            }
            if tokens.contains(&0) {
                let n = used.iter().filter(|v| !**v).count();
                let mut heads = HashMap::new();
                heads.try_reserve(n)?;
                let mut links = Vec::new();
                links.try_reserve_exact(n)?;
                for j in (0..previous).rev().filter(|&j| !used[j]) {
                    let next = heads.insert(
                        (old.positions.record(j), old.positions.surface(j)),
                        links.len(),
                    );
                    links.push((j, next));
                }
                for (i, t) in tokens.iter_mut().enumerate().filter(|(_, v)| **v == 0) {
                    let key = (positions.record(i), positions.surface(i));
                    if let Some(link) = heads.remove(&key) {
                        let (j, next) = links[link];
                        if let Some(next) = next {
                            heads.insert(key, next);
                        }
                        *t = old.token_at(j);
                        used[j] = true;
                    }
                }
                let mut owners = HashMap::new();
                owners.try_reserve(used.iter().filter(|v| !**v).count())?;
                for j in (0..previous).filter(|&j| !used[j]) {
                    owners
                        .entry(old.positions.record(j))
                        .and_modify(|v: &mut (Option<usize>, usize)| v.0 = None)
                        .or_insert((Some(j), 0));
                }
                for (i, _) in tokens.iter().enumerate().filter(|(_, v)| **v == 0) {
                    if let Some(v) = owners.get_mut(&positions.record(i)) {
                        v.1 += 1;
                    }
                }
                for (i, t) in tokens.iter_mut().enumerate().filter(|(_, v)| **v == 0) {
                    *t = if let Some(&(Some(j), 1)) = owners.get(&positions.record(i)) {
                        old.token_at(j)
                    } else {
                        ids.0.next().expect("预留了整个区域的身份数")
                    };
                }
            }
            first = tokens.first().copied().unwrap_or(first);
            if tokens.windows(2).all(|t| t[0].checked_add(1) == Some(t[1])) {
                tokens = Vec::new();
            }
        }
        let serial = crate::NEXT_OWNER
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |n| n.checked_add(1),
            )
            .map_err(|_| Error::Capacity)?;
        Ok(Self {
            serial,
            positions,
            tokens,
            first,
        })
    }

    /// 此不可变 CPU 包的所有者身份，移动后仍有效。仅用于续传，不用作三角形的跨帧身份。
    pub const fn serial(&self) -> u64 {
        self.serial
    }
    /// 可直接构建 BLAS 的压缩位置流。
    pub const fn positions(&self) -> &PackedQuads {
        &self.positions
    }
    fn token_at(&self, i: usize) -> u64 {
        if self.tokens.is_empty() {
            self.first + i as u64
        } else {
            self.tokens[i]
        }
    }
    /// 源序号的完整驻留身份。连续新区域只在 CPU 和线上保存一个首值。
    pub fn token(&self, i: usize) -> Option<u64> {
        (i < self.positions.headers.len()).then(|| self.token_at(i))
    }
    /// GPU 展开的起始身份。显式身份列存在时着色器改用该列。
    pub const fn first_token(&self) -> u64 {
        self.first
    }
    /// 只有不连续的身份才拥有逐面表，连续区域返回空切片。
    pub const fn explicit_tokens(&self) -> &[u64] {
        self.tokens.as_slice()
    }
    /// 连续身份可用首值加源序号恢复。
    pub const fn consecutive(&self) -> bool {
        self.tokens.is_empty()
    }
    /// 当前记录独占的堆容量。
    pub const fn retained_bytes(&self) -> usize {
        self.positions.retained_bytes() + self.tokens.capacity() * 8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cache, Prepared};

    #[test]
    fn coincident_surfaces_follow_their_own_material_after_reordering() -> Result<(), Error> {
        let owner = Cache::<()>::new()?;
        let mut update = owner.begin(0)?;
        let ids = update.parts_mut().2;
        let make = |materials: [u32; 2]| -> Result<PackedQuads, Error> {
            let mut encoder = QuadEncoder::with_capacity(2)?;
            for material in materials {
                let mut attr = [[0; 4]; 3];
                attr[2][3] = material;
                encoder.push(
                    [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
                    0,
                    attr,
                )?;
            }
            Ok(encoder.finish())
        };
        let old = ResidentQuads::prepare(make([5, 9])?, None, ids)?;
        let new = ResidentQuads::prepare(make([9, 5])?, Some(&old), ids)?;
        assert_eq!([new.token(0), new.token(1)], [old.token(1), old.token(0)]);
        Ok(())
    }

    #[test]
    fn fallback_faces_keep_their_geometry_and_identity_when_repacked() -> Result<(), Error> {
        let owner = Cache::<()>::new()?;
        let mut update = owner.begin(0)?;
        let ids = update.parts_mut().2;
        let mut source = QuadEncoder::with_capacity(2)?;
        for x in [-0.5, 0.1] {
            source.push(
                [[x, 0., 0.], [1., 0., 0.], [1., 1., 0.], [x, 1., 0.]],
                0,
                [[0; 4]; 3],
            )?;
        }
        let old = ResidentQuads::prepare(source.finish(), None, ids)?;
        let mut encoder = QuadEncoder::with_capacity(2)?;
        encoder.rebind(&old, 1, [[17; 4]; 3])?;
        encoder.extend_packed(old.positions(), 0..1)?;
        let new = ResidentQuads::prepare(encoder.finish(), Some(&old), ids)?;
        for (i, prior) in [1, 0].into_iter().enumerate() {
            assert_eq!(new.positions().record(i), old.positions().record(prior));
            assert_eq!(new.token(i), old.token(prior));
        }
        Ok(())
    }

    #[test]
    fn full_width_boundary_and_reversed_corner_keep_the_eight_byte_record() -> Result<(), Error> {
        let mut enc = QuadEncoder::with_capacity(2)?;
        let quad = [
            [64., 64., 64.],
            [64., 0., 64.],
            [0., 0., 64.],
            [0., 64., 64.],
        ];
        // 负法线在最高边界没有 owner 格；反向绕序后法线从 z=63 的格指向 z=64。
        let [a, b, c, d] = quad;
        enc.push([a, d, c, b], 0, [[0; 4]; 3])?;
        enc.push([a, d, c, b], 0, [[0; 4]; 3])?;
        let stream = enc.finish();
        assert!(stream.payload().is_empty());
        assert_eq!(stream.bytes(), 2 * 8 + 64);
        assert_eq!(
            stream.headers()[0][0] & ((1 << 31) - 1),
            (63 << 12) | (63 << 18) | (63 << 24) | (1 << 30)
        );
        Ok(())
    }

    #[test]
    fn unloaded_face_does_not_recover_a_retired_token() -> Result<(), Error> {
        let mut owner = Cache::new()?;
        let make = || -> Result<PackedQuads, Error> {
            let mut encoder = QuadEncoder::with_capacity(1)?;
            encoder.push(
                [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
                0,
                [[0; 4]; 3],
            )?;
            Ok(encoder.finish())
        };
        let mut update = owner.begin(1)?;
        let (_, entries, ids) = update.parts_mut();
        let first = ResidentQuads::prepare(make()?, None, ids)?;
        let retired = first.token(0).unwrap();
        entries.push(Prepared::Built(first));
        owner.commit(update, |_| 0..0)?;
        let empty = owner.begin(0)?;
        owner.commit(empty, |_| 0..0)?;
        let mut update = owner.begin(1)?;
        let next = ResidentQuads::prepare(make()?, None, update.parts_mut().2)?;
        assert_ne!(retired, next.token(0).unwrap());
        Ok(())
    }
}
