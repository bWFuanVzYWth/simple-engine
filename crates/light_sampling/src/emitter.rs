//! 静态发光纹理的有界texel裁剪与面积提案；只用于资源准备，不按实例或帧构建。
//! 参考旧Prime emission_alias.rs的重心多边形裁剪与表布局，统一使用AliasTable的整数质量。
use crate::{AliasTable, Error, QuadArea, SELECTORS};

/// 纹理内alias的准备上限；更大的资源合法使用均匀面积fallback。
pub const MAX_TEXELS: usize = 256 * 256;
const HEADER_WORDS: usize = 8;
const ENTRY_WORDS: usize = 8;
const PATCH_WORDS: usize = 8;

/// 遍历归一化 UV 三角形覆盖的 texel，并报告该 texel 占三角形的面积比例。
///
/// 输入须为 `[0,1]` 内的非退化三角形，尺寸非零且 texel 数不超过 [`MAX_TEXELS`]。
/// 访问顺序为从上到下、从左到右，只报告正面积，不分配内存。
///
/// # Errors
/// 输入超出上述域时返回 [`Error::Emission`]，且不调用 `visit`。
pub fn triangle_texel_fractions(
    uvs: [[f32; 2]; 3],
    extent: [u32; 2],
    mut visit: impl FnMut([u32; 2], f64),
) -> Result<(), Error> {
    if extent.contains(&0)
        || u64::from(extent[0]) * u64::from(extent[1]) > MAX_TEXELS as u64
        || uvs.iter().flatten().any(|v| !(0.0..=1.0).contains(v))
    {
        return Err(Error::Emission);
    }
    let uv = uvs.map(|point| point.map(f64::from));
    if determinant(sub(uv[1], uv[0]), sub(uv[2], uv[0])) == 0.0 {
        return Err(Error::Emission);
    }
    let bounds = std::array::from_fn::<_, 2, _>(|axis| {
        let size = f64::from(extent[axis]);
        let lo = (uv.iter().map(|p| p[axis]).fold(1.0, f64::min) * size).floor() as u32;
        let hi = (uv.iter().map(|p| p[axis]).fold(0.0, f64::max) * size).ceil() as u32;
        lo..hi
    });
    let mut polygon = Polygon::default();
    let mut scratch = Polygon::default();
    for y in bounds[1].clone() {
        for x in bounds[0].clone() {
            polygon.clip_triangle(uv, extent, [x, y], &mut scratch);
            let fraction = polygon
                .patches()
                .map(|bary| determinant(sub(bary[1], bary[0]), sub(bary[2], bary[0])).abs())
                .sum::<f64>();
            if fraction > 0.0 {
                visit([x, y], fraction);
            }
        }
    }
    Ok(())
}

/// 应用已经解析的覆盖规则；不把透明介质的alpha误当孔洞。
#[derive(Clone, Copy, Debug)]
pub enum Coverage {
    /// 表面覆盖完整几何，包括不把alpha用作覆盖的透射界面。
    Opaque,
    /// alpha达到给定的8bit阈值才可能发光；动画输入须给出全帧alpha上界。
    AlphaThreshold(u8),
}

/// 独占静态源提案；应用先提供完整动画包络的线性亮度权重与coverage alpha上界。
/// 权重不要求等于真实功率，但任何可能发光的texel都须有正支持。
#[derive(Debug)]
pub struct EmissionProposal {
    extent: [u32; 2],
    weights: Vec<f32>,
    alpha: Vec<u8>,
}
impl EmissionProposal {
    /// 复制一份资源局部提案，不保留输入借用或引用计数。
    ///
    /// extent须非零，两个切片各为width*height项，权重须有限非负。
    /// 超过MAX_TEXELS、全黑或无alpha变化的低对比(peak<2*minimum)输入返回None，
    /// 保留均匀面积策略；调用方可先检查MAX_TEXELS以避免生成超大临时数组。
    /// # Errors
    /// 输入布局/权重无效或分配失败返回对应错误。
    pub fn new(extent: [u32; 2], weights: &[f32], alpha: &[u8]) -> Result<Option<Self>, Error> {
        let count = u64::from(extent[0]) * u64::from(extent[1]);
        if extent.contains(&0) || count != weights.len() as u64 || weights.len() != alpha.len() {
            return Err(Error::Emission);
        }
        if weights.iter().any(|w| !w.is_finite() || *w < 0.0) {
            return Err(Error::Emission);
        }
        if weights.len() > MAX_TEXELS {
            return Ok(None);
        }
        let mut minimum = f32::INFINITY;
        let mut maximum = 0.0_f32;
        for &weight in weights {
            minimum = minimum.min(weight);
            maximum = maximum.max(weight);
        }
        if maximum == 0.0
            || (minimum > 0.0
                && f64::from(maximum) < 2.0 * f64::from(minimum)
                && alpha.iter().all(|a| *a == alpha[0]))
        {
            return Ok(None);
        }
        let mut owned_weights = Vec::new();
        owned_weights.try_reserve_exact(weights.len())?;
        owned_weights.extend_from_slice(weights);
        let mut owned_alpha = Vec::new();
        owned_alpha.try_reserve_exact(alpha.len())?;
        owned_alpha.extend_from_slice(alpha);
        Ok(Some(Self {
            extent,
            weights: owned_weights,
            alpha: owned_alpha,
        }))
    }
    /// 与本提案共同建立的源tile尺寸；UV输入使用此tile的局部`[0,1]`域。
    pub const fn extent(&self) -> [u32; 2] {
        self.extent
    }
    fn texel(&self, index: usize) -> (f64, u8) {
        (f64::from(self.weights[index]), self.alpha[index])
    }
}

/// 一份静态sourcequad的完整GPU表；不存材质、纹理句柄或场景身份。
#[derive(Debug)]
pub struct EmitterAlias {
    words: Vec<u32>,
}
impl EmitterAlias {
    /// 只在首次/重载资源准备中构建，结果可按UV、面积和覆盖规则在资源世代内intern。
    ///
    /// vertices为真实012/023两片，uv为源tile局部坐标。当前纹理重要性提案要求单一可逆仿射映射；
    /// 非共面、折叠/重复UV或不在`[0,1]`的来源返回None，由任意quad的均匀面积策略承接。
    /// 完整退化几何也无提案；其他无效f32几何返回Error，不伪造可采样面。
    /// 空支持或密集低对比(覆盖面积>=75%、peak<2*mean)来源返回None。
    /// 运行期仅查询本表，不构造或追加，纹理/alpha实值仍由应用在采样点重新求值。
    /// # Errors
    /// 无效输入、表偏移超出u32或分配失败返回对应错误。
    pub fn build(
        vertices: [[f32; 3]; 4],
        uvs: [[f32; 2]; 4],
        coverage: Coverage,
        proposal: &EmissionProposal,
    ) -> Result<Option<Self>, Error> {
        if uvs.iter().flatten().any(|value| !value.is_finite()) {
            return Err(Error::Emission);
        }
        let geometry = match QuadArea::new(&vertices) {
            Ok(geometry) => geometry,
            Err(Error::Quad)
                if vertices.iter().flatten().all(|v| v.is_finite()) && degenerate(vertices) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let areas = geometry.areas();
        if !affine(vertices, uvs, areas) {
            return Ok(None);
        }
        let total_area = geometry.area();
        let [w, h] = proposal.extent;
        let lo: [u32; 2] = std::array::from_fn(|axis| {
            (uvs.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min)
                * proposal.extent[axis] as f32)
                .floor() as u32
        });
        let hi: [u32; 2] = std::array::from_fn(|axis| {
            (uvs.iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max)
                * proposal.extent[axis] as f32)
                .ceil() as u32
        });
        let mut cells = Vec::new();
        let mut patches = Vec::new();
        let mut polygon = Polygon::default();
        let mut clipped = Polygon::default();
        let uv = uvs.map(|point| point.map(f64::from));
        let triangles = [[uv[0], uv[1], uv[2]], [uv[0], uv[2], uv[3]]];
        for y in lo[1]..hi[1].min(h) {
            for x in lo[0]..hi[0].min(w) {
                let texel = (y * w + x) as usize;
                let (weight, alpha) = proposal.texel(texel);
                if weight == 0.0
                    || matches!(coverage, Coverage::AlphaThreshold(minimum) if alpha < minimum)
                {
                    continue;
                }
                let first_patch = patches.len();
                for (half, &area) in areas.iter().enumerate() {
                    if area == 0.0 {
                        continue;
                    }
                    polygon.clip_triangle(triangles[half], proposal.extent, [x, y], &mut clipped);
                    for bary in polygon.patches() {
                        let clipped_area =
                            area * determinant(sub(bary[1], bary[0]), sub(bary[2], bary[0])).abs();
                        if clipped_area > 0.0 {
                            let stored_area = clipped_area as f32;
                            if stored_area <= 0.0 || !stored_area.is_finite() {
                                return Ok(None);
                            }
                            patches.try_reserve(1)?;
                            patches.push(Patch {
                                half: half as u32,
                                area: clipped_area,
                                bary,
                            });
                        }
                    }
                }
                let area = patches[first_patch..].iter().map(|p| p.area).sum::<f64>();
                if area > 0.0 {
                    cells.try_reserve(1)?;
                    cells.push(Cell {
                        texel,
                        weight,
                        area,
                        patches: first_patch..patches.len(),
                    });
                }
            }
        }
        if cells.is_empty() {
            return Ok(None);
        }
        let support_area = cells.iter().map(|c| c.area).sum::<f64>();
        let mut weights = Vec::new();
        weights.try_reserve_exact(cells.len())?;
        weights.extend(cells.iter().map(|c| c.weight * c.area));
        let mean = weights.iter().sum::<f64>() / total_area;
        let peak = cells.iter().map(|c| c.weight).fold(0.0, f64::max);
        if support_area >= total_area * 0.75 && peak < mean * 2.0 {
            return Ok(None);
        }
        let alias = AliasTable::build(&weights)?;
        let columns = alias.entries().len();
        let grid_words = w as usize * h as usize;
        let entry_offset = HEADER_WORDS + grid_words;
        let patch_offset = entry_offset + columns * ENTRY_WORDS;
        let length = patch_offset
            .checked_add(patches.len() * PATCH_WORDS)
            .ok_or(Error::Capacity)?;
        if length > u32::MAX as usize {
            return Err(Error::Capacity);
        }
        let mut words = Vec::new();
        words.try_reserve_exact(length)?;
        words.resize(patch_offset, 0);
        words[..HEADER_WORDS].copy_from_slice(&[
            w,
            h,
            columns as u32 - 1,
            alias.column_shift(),
            HEADER_WORDS as u32,
            entry_offset as u32,
            patch_offset as u32,
            cells.len() as u32,
        ]);
        words[HEADER_WORDS..entry_offset].fill(u32::MAX);
        for (i, entry) in alias.entries().iter().enumerate() {
            let at = entry_offset + i * ENTRY_WORDS;
            words[at..at + 3].copy_from_slice(&entry[..3]);
        }
        for (i, cell) in cells.into_iter().enumerate() {
            words[HEADER_WORDS + cell.texel] = i as u32;
            let at = entry_offset + i * ENTRY_WORDS;
            words[at + 3] = ((words.len() - patch_offset) / PATCH_WORDS) as u32;
            words[at + 4] = cell.patches.len() as u32;
            let density =
                (f64::from(alias.entries()[i][2]) / f64::from(SELECTORS) / cell.area) as f32;
            if !density.is_finite() || density <= 0.0 {
                return Ok(None);
            }
            words[at + 5] = density.to_bits();
            words[at + 6] = (cell.area as f32).to_bits();
            for patch in &patches[cell.patches] {
                words.extend([patch.half, (patch.area as f32).to_bits()]);
                words.extend(
                    patch
                        .bary
                        .into_iter()
                        .flatten()
                        .map(|value| (value as f32).to_bits()),
                );
            }
        }
        Ok(Some(Self { words }))
    }
    /// 32B header、完整texel反查、32B alias列、32B重心patch；所有offset相对header首字。
    /// `header=[width,height,columnMask,columnShift,gridOffset,entryOffset,patchOffset,cellCount]`。
    /// `entry=[threshold,alias,mass,firstPatch,patchCount,asuint(density),asuint(area),0]`。
    /// `patch=[half,asuint(area),a.xy,b.xy,c.xy]`；应用拼接时只另存header的全局word偏移。
    pub fn words(&self) -> &[u32] {
        &self.words
    }
}

#[derive(Clone, Copy, Default)]
struct Vertex {
    uv: [f64; 2],
    bary: [f64; 2],
}
struct Patch {
    half: u32,
    area: f64,
    bary: [[f64; 2]; 3],
}
struct Cell {
    texel: usize,
    weight: f64,
    area: f64,
    patches: std::ops::Range<usize>,
}

// Triangle ∩ rectangle has at most seven distinct vertices. Keep boundary duplicates in
// the original arithmetic order: even without convexity, four clips take at most
// floor(3n/2) records each, giving 3 -> 4 -> 6 -> 9 -> 13.
#[derive(Default)]
struct Polygon {
    vertices: [Vertex; 13],
    len: usize,
}
impl Polygon {
    fn push(&mut self, vertex: Vertex) {
        self.vertices[self.len] = vertex;
        self.len += 1;
    }

    fn clip_triangle(
        &mut self,
        uv: [[f64; 2]; 3],
        extent: [u32; 2],
        texel: [u32; 2],
        scratch: &mut Self,
    ) {
        self.len = 3;
        for (i, (uv, bary)) in uv
            .into_iter()
            .zip([[0., 0.], [1., 0.], [0., 1.]])
            .enumerate()
        {
            self.vertices[i] = Vertex { uv, bary };
        }
        for (axis, value) in texel.into_iter().enumerate() {
            let size = f64::from(extent[axis]);
            clip(self, scratch, axis, f64::from(value) / size, true);
            clip(scratch, self, axis, f64::from(value + 1) / size, false);
        }
    }

    fn patches(&self) -> impl Iterator<Item = [[f64; 2]; 3]> {
        (1..self.len.saturating_sub(1)).map(|i| {
            [
                self.vertices[0].bary,
                self.vertices[i].bary,
                self.vertices[i + 1].bary,
            ]
        })
    }
}

fn determinant(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn clip(input: &Polygon, output: &mut Polygon, axis: usize, edge: f64, above: bool) {
    output.len = 0;
    if input.len == 0 {
        return;
    }
    let inside = |v: Vertex| {
        if above {
            v.uv[axis] >= edge
        } else {
            v.uv[axis] <= edge
        }
    };
    let mut a = input.vertices[input.len - 1];
    for &b in &input.vertices[..input.len] {
        if inside(a) != inside(b) {
            let t = (edge - a.uv[axis]) / (b.uv[axis] - a.uv[axis]);
            output.push(Vertex {
                uv: std::array::from_fn(|i| a.uv[i] + t * (b.uv[i] - a.uv[i])),
                bary: std::array::from_fn(|i| a.bary[i] + t * (b.bary[i] - a.bary[i])),
            });
        }
        if inside(b) {
            output.push(b);
        }
        a = b;
    }
}

fn affine(vertices: [[f32; 3]; 4], uvs: [[f32; 2]; 4], areas: [f64; 2]) -> bool {
    if uvs
        .iter()
        .flatten()
        .any(|&v| !v.is_finite() || !(0.0..=1.0).contains(&v))
    {
        return false;
    }
    let uv = uvs.map(|p| p.map(f64::from));
    let a = sub(uv[1], uv[0]);
    let b = sub(uv[2], uv[0]);
    let d = determinant(a, b);
    if d == 0. {
        return false;
    }
    let c = sub(uv[3], uv[0]);
    let s = determinant(c, b) / d;
    let t = determinant(a, c) / d;
    let positions = vertices.map(|p| p.map(f64::from));
    let scale = areas.into_iter().sum::<f64>().sqrt().max(1e-12);
    if (0..3).any(|i| {
        let predicted = positions[0][i]
            + s * (positions[1][i] - positions[0][i])
            + t * (positions[2][i] - positions[0][i]);
        (predicted - positions[3][i]).abs() > scale * 1e-6
    }) {
        return false;
    }
    let d1 = determinant(sub(uv[3], uv[2]), sub(uv[0], uv[2]));
    // A collapsed second half is the declared independent-triangle representation.
    d1 == 0. && vertices[3] == vertices[2] || d1 * d > 0.
}

fn degenerate(vertices: [[f32; 3]; 4]) -> bool {
    [[0, 1, 2], [0, 2, 3]].into_iter().all(|indices| {
        let [a, b, c] = indices.map(|i| vertices[i].map(f64::from));
        let u = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
        let v = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]
        .into_iter()
        .all(|v| v == 0.0)
    })
}
