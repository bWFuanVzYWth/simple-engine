//! 缓存 64 叉源树的二倍金字塔，再构建固定 64³ 的稀疏渲染树。
//!
//! 占据计数是精确的源格数量，不是面积或可见覆盖。代表材料按优先级、材料编号
//! 确定性选择，不宣称多数材料；六方向覆盖采用前后合成再平均的固定层级近似。
//! 所有归约只读取八个子摘要，不重新扫描其细体素。每个原生节点缓存八个奇数层
//! 摘要和一个偶数层摘要；初次访问的成本取决于未缓存的唯一节点数。
//!
//! 分类策略须在缓存有效期内保持不变；改变策略后调用 [`Pyramid::clear`]。
//! 生成树的叶是摘要编号，仅供同一 [`Pyramid`] 解析，不是游戏材料编号。

use std::{
    convert::Infallible,
    hash::{DefaultHasher, Hash, Hasher},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    Direction, Quad,
    sparse::{self, Cell, Node, Tree},
};

#[path = "lod_summary.rs"]
mod summary;
pub use summary::{Face, Sample, Summary};

const WHOLE: u8 = 8;
static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// 源节点所属实例与该槽内容的有效代号。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Identity {
    /// 源实例身份；不同实例不得相同。
    pub owner: u64,
    /// 槽复用或内容变化后必须改变；不可在缓存仍有效时回绕。
    pub generation: u64,
}

/// 具有稳定内容身份的只读 64 叉源树。
pub trait Source: Tree {
    /// 返回节点身份；同实例、层级、槽及代号相同就必须表示相同不可变内容。
    ///
    /// 保留底层读取错误；源节点和根由调用方保活，不由缓存隐式持有。
    fn identity(&self, index: u32, level: u8) -> Result<Identity, Self::Error>;
}

/// 根内按二倍网格对齐的源区域。
#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// 覆盖源根的值或节点；节点根的 `root_lod` 必须为正偶数。
    pub root: Cell,
    /// 源根边长的二进制指数，最多 42。
    pub root_lod: u8,
    /// 请求区域在根内的最小角，须按 `2^lod` 对齐。
    pub minimum: [u64; 3],
    /// 请求区域边长的二进制指数。
    pub lod: u8,
}

/// 生成树或摘要编号访问失败。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AccessError {
    /// 生成树尚未完成，或所属金字塔已清空、不同。
    #[error("LOD 树的摘要版本已失效")]
    Stale,
    /// 层、节点或坐标越界。
    #[error("LOD 树访问越界")]
    Bounds,
}

/// 输入、源读取、内存或表面提取失败；保留底层来源。
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    /// 源节点读取或身份查询失败。
    #[error("LOD 源读取失败")]
    Read(#[source] E),
    /// 区域不对齐、超出源根、层数不合法或邻居尺度不一致。
    #[error("LOD 区域或层级无效")]
    Region,
    /// 最细源层仍出现节点指针。
    #[error("LOD 源指针超过声明深度")]
    Depth,
    /// 摘要或生成节点数量超过 u32。
    #[error("LOD 摘要或节点编号耗尽")]
    Capacity,
    /// 金字塔所有者身份或批次版本已耗尽，不能再发行不会混淆的新身份。
    #[error("LOD 所有者身份或批次版本耗尽")]
    IdentityExhausted,
    /// 生成树不属于当前摘要版本。
    #[error(transparent)]
    Access(#[from] AccessError),
    /// 内存申请失败。
    #[error("LOD 缓冲分配失败")]
    Allocation(#[from] std::collections::TryReserveError),
    /// 稀疏界面提取失败。
    #[error("LOD 表面提取失败")]
    Mesh(#[from] sparse::Error<AccessError>),
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct Key {
    identity: Identity,
    index: u32,
    level: u8,
}

struct Frame {
    key: Key,
    cells: [Cell; 64],
    values: [Summary; 64],
    next: usize,
}

struct Cached {
    key: Key,
    values: [Summary; 9],
}

#[derive(Clone, Copy)]
struct PaletteSlot {
    revision: u64,
    index: u32,
}

#[derive(Clone, Copy)]
struct View {
    cell: Cell,
    lod: u8,
    group: u8,
}

struct BuildFrame {
    key: BuildKey,
    children: [View; 64],
    values: [Cell; 64],
    next: usize,
    level: u8,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct BuildKey {
    source: Key,
    group: u8,
    level: u8,
}

#[derive(Clone, Copy)]
struct BuildSlot {
    revision: u64,
    key: BuildKey,
    cell: Cell,
}

struct OwnedNode {
    values: [u32; 64],
    pointers: u64,
}

/// 一份 64³ 粗网格的稀疏树；保留分配容量，叶由所属金字塔解析。
pub struct LodTree {
    nodes: [Vec<OwnedNode>; 3],
    root: Cell,
    owner: u64,
    revision: u64,
    lod: u8,
    valid: bool,
}

impl Default for LodTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LodTree {
    /// 创建空结果槽，不分配节点。
    pub fn new() -> Self {
        Self {
            nodes: std::array::from_fn(|_| Vec::new()),
            root: Cell::Uniform(0),
            owner: 0,
            revision: 0,
            lod: 0,
            valid: false,
        }
    }

    /// 每粗格相对于源格的边长指数。
    pub const fn lod(&self) -> u8 {
        self.lod
    }

    /// 节点后备容量与结果槽的字节数，不含摘要缓存及 allocator 元数据。
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .nodes
                .iter()
                .map(|nodes| nodes.capacity() * size_of::<OwnedNode>())
                .sum::<usize>()
    }

    /// 借用生成树，叶仅可作为所属金字塔的摘要编号使用。
    pub fn block(&self) -> sparse::Block<'_, Self> {
        sparse::Block::new(self, self.root)
    }

    /// 整个粗网格共享同一摘要时常数时间返回，供空块和均匀边界快速路径使用。
    /// # Errors
    /// 结果已过期或摘要编号无效时返回错误。
    pub fn uniform<'a>(&self, pyramid: &'a Pyramid) -> Result<Option<&'a Summary>, AccessError> {
        self.check(pyramid)?;
        let Cell::Uniform(index) = self.root else {
            return Ok(None);
        };
        pyramid
            .summaries
            .get(index as usize)
            .map(Some)
            .ok_or(AccessError::Bounds)
    }

    /// 在 64³ 粗格坐标读取摘要，供共享边界对齐。
    pub fn sample<'a>(
        &self,
        pyramid: &'a Pyramid,
        coord: [u8; 3],
    ) -> Result<&'a Summary, AccessError> {
        self.check(pyramid)?;
        if coord.iter().any(|&value| value >= 64) {
            return Err(AccessError::Bounds);
        }
        let mut cell = self.root;
        for level in (0..3).rev() {
            let Cell::Node(index) = cell else { break };
            let shift = level * 2;
            let x = (coord[0] >> shift) & 3;
            let y = (coord[1] >> shift) & 3;
            let z = (coord[2] >> shift) & 3;
            let slot = (x & 1)
                | ((y & 1) << 1)
                | ((z & 1) << 2)
                | ((x & 2) << 2)
                | ((y & 2) << 3)
                | ((z & 2) << 4);
            cell = self.node(index, level as u8)?.child(slot);
        }
        let Cell::Uniform(index) = cell else {
            return Err(AccessError::Bounds);
        };
        pyramid
            .summaries
            .get(index as usize)
            .ok_or(AccessError::Bounds)
    }

    fn check(&self, pyramid: &Pyramid) -> Result<(), AccessError> {
        if !self.valid || self.owner != pyramid.owner || self.revision != pyramid.revision {
            Err(AccessError::Stale)
        } else {
            Ok(())
        }
    }
}

impl Tree for LodTree {
    type Error = AccessError;
    fn node(&self, index: u32, level: u8) -> Result<Node<'_>, Self::Error> {
        let node = self
            .nodes
            .get(level as usize)
            .and_then(|nodes| nodes.get(index as usize))
            .ok_or(AccessError::Bounds)?;
        Ok(Node::new(&node.values, node.pointers))
    }
}

/// 调用方持有的摘要缓存和构建 scratch；不拥有源根，不隐藏调度。
pub struct Pyramid {
    owner: u64,
    revision: u64,
    cache: Vec<Option<Cached>>,
    cache_len: usize,
    palette: Vec<Option<PaletteSlot>>,
    summaries: Vec<Summary>,
    frames: Vec<Frame>,
    build_frames: Vec<BuildFrame>,
    build_slots: Vec<Option<BuildSlot>>,
    build_revision: u64,
}

impl Pyramid {
    /// 创建独占缓存，初始只保存一项空摘要；身份耗尽或分配失败返回来源。
    pub fn new() -> Result<Self, Error<Infallible>> {
        Self::with_capacity(4096)
    }

    /// 预分配指定数量的 direct-mapped 原生摘要槽；容量只影响命中率。
    ///
    /// 冲突仅替换一个缓存槽，不重建表、不修改生成树的 palette。零容量禁用缓存。
    /// 身份耗尽、容量溢出或预分配失败返回错误。
    pub fn with_capacity(max_nodes: usize) -> Result<Self, Error<Infallible>> {
        let owner = NEXT_OWNER
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| Error::IdentityExhausted)?;
        let mut cache = Vec::new();
        cache.try_reserve_exact(max_nodes)?;
        cache.resize_with(max_nodes, || None);
        let mut palette = Vec::new();
        palette.try_reserve_exact(max_nodes.max(256))?;
        palette.resize(max_nodes.max(256), None);
        let mut summaries = Vec::new();
        summaries.try_reserve_exact(1)?;
        summaries.push(Summary::EMPTY);
        let mut build_slots = Vec::new();
        build_slots.try_reserve_exact(max_nodes.max(256))?;
        build_slots.resize(max_nodes.max(256), None);
        Ok(Self {
            owner,
            revision: 0,
            cache,
            cache_len: 0,
            palette,
            summaries,
            frames: Vec::new(),
            build_frames: Vec::new(),
            build_slots,
            build_revision: 0,
        })
    }

    /// 清除逻辑缓存并保留容量，使旧生成树失效；版本耗尽返回错误并保留原状态。
    pub fn clear(&mut self) -> Result<(), Error<Infallible>> {
        self.begin_batch()?;
        for slot in &mut self.cache {
            *slot = None;
        }
        self.cache_len = 0;
        Ok(())
    }

    /// 当前缓存的不同原生源节点数量，不包含均匀区域。
    pub fn cached_nodes(&self) -> usize {
        self.cache_len
    }

    /// 在前批生成树已消费后重置临时 palette，源摘要缓存始终保留。
    ///
    /// 旧树失效。palette 槽用版本懒失效，不扫描清表；POD 摘要只重置 Vec 长度。
    /// 必须在一组中心与邻居构建之前调用，不能在这些树之间调用。
    /// 版本耗尽时返回错误，不改变旧树和缓存。
    pub fn begin_batch(&mut self) -> Result<(), Error<Infallible>> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::IdentityExhausted)?;
        self.frames.clear();
        self.build_frames.clear();
        self.summaries.clear();
        self.summaries.push(Summary::EMPTY);
        Ok(())
    }

    /// 缓存、临时 palette 和 scratch 的保留字节，不含 allocator 元数据。
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.summaries.capacity() * size_of::<Summary>()
            + self.cache.capacity() * size_of::<Option<Cached>>()
            + self.palette.capacity() * size_of::<Option<PaletteSlot>>()
            + self.frames.capacity() * size_of::<Frame>()
            + self.build_frames.capacity() * size_of::<BuildFrame>()
            + self.build_slots.capacity() * size_of::<Option<BuildSlot>>()
    }

    /// 查询一个区域的摘要；分类规则必须与缓存中已有条目一致。
    pub fn summarize<T: Source + ?Sized>(
        &mut self,
        source: &T,
        region: Region,
        classify: impl Fn(u32) -> Sample,
    ) -> Result<Summary, Error<T::Error>> {
        validate(region)?;
        let view = locate(source, region)?;
        self.summary_view(source, view, &classify)
    }

    /// 从源区域构建 64³ 粗格稀疏树，区域边长须至少为 64。
    ///
    /// 均匀子区域直接形成叶，不展开细格。成功树持有当前摘要版本；输入校验失败
    /// 保留旧结果，开始构建后失败则结果失效，缓存中已完成的归约仍可复用。
    pub fn build<T: Source + ?Sized>(
        &mut self,
        source: &T,
        region: Region,
        classify: impl Fn(u32) -> Sample,
        output: &mut LodTree,
    ) -> Result<(), Error<T::Error>> {
        validate(region)?;
        if region.lod < 6 {
            return Err(Error::Region);
        }
        let view = locate(source, region)?;
        output.valid = false;
        for nodes in &mut output.nodes {
            nodes.clear();
        }
        self.build_frames.clear();
        self.build_revision = self.build_revision.checked_add(1).ok_or(Error::Capacity)?;
        let coarse_lod = region.lod - 6;
        let root = if let Cell::Uniform(value) = view.cell {
            Cell::Uniform(self.intern(Summary::uniform_value(
                value,
                coarse_lod,
                classify(value),
            ))?)
        } else {
            self.push_build(source, view, 2)?;
            loop {
                let frame = self.build_frames.last().expect("构建帧非空");
                if frame.next == 64 {
                    let frame = self.build_frames.pop().expect("已检查帧");
                    let cell = emit_node(output, frame.level, frame.values)?;
                    let slot = hash(&frame.key) % self.build_slots.len();
                    self.build_slots[slot] = Some(BuildSlot {
                        revision: self.build_revision,
                        key: frame.key,
                        cell,
                    });
                    if let Some(parent) = self.build_frames.last_mut() {
                        parent.values[parent.next] = cell;
                        parent.next += 1;
                    } else {
                        break cell;
                    }
                } else {
                    let child = frame.children[frame.next];
                    let level = frame.level;
                    let result = if let Cell::Uniform(value) = child.cell {
                        Some(Cell::Uniform(self.intern(Summary::uniform_value(
                            value,
                            coarse_lod,
                            classify(value),
                        ))?))
                    } else if level == 0 {
                        let summary = self.summary_view(source, child, &classify)?;
                        Some(Cell::Uniform(self.intern(summary)?))
                    } else {
                        let key = build_key(source, child, level - 1)?;
                        self.build_slots[hash(&key) % self.build_slots.len()]
                            .filter(|entry| {
                                entry.revision == self.build_revision && entry.key == key
                            })
                            .map(|entry| entry.cell)
                    };
                    if let Some(cell) = result {
                        let frame = self.build_frames.last_mut().expect("父帧仍有效");
                        frame.values[frame.next] = cell;
                        frame.next += 1;
                    } else {
                        self.push_build(source, child, level - 1)?;
                    }
                }
            }
        };
        output.root = root;
        output.lod = coarse_lod;
        output.owner = self.owner;
        output.revision = self.revision;
        output.valid = true;
        Ok(())
    }

    /// 按摘要和实际方向提取面；禁用边界供上层混合尺度接口替换。
    ///
    /// 邻居须属于本缓存且同尺度。策略可读取两侧的方向材料和覆盖，返回最终游戏标签。
    pub fn mesh_by(
        &self,
        center: &LodTree,
        neighbors: [Option<&LodTree>; 6],
        boundaries: [bool; 6],
        scratch: &mut sparse::Scratch,
        output: &mut Vec<Quad>,
        mut face: impl FnMut(&Summary, &Summary, Direction) -> Option<u32>,
    ) -> Result<(), Error<Infallible>> {
        center.check(self)?;
        for neighbor in neighbors.into_iter().flatten() {
            neighbor.check(self)?;
            if neighbor.lod != center.lod {
                return Err(Error::Region);
            }
        }
        sparse::mesh_by_direction_boundaries(
            center.block(),
            neighbors.map(|tree| tree.map(LodTree::block)),
            boundaries,
            scratch,
            output,
            |a, b, d| face(&self.summaries[a as usize], &self.summaries[b as usize], d),
        )?;
        Ok(())
    }

    fn intern<E>(&mut self, summary: Summary) -> Result<u32, Error<E>> {
        let slot = hash(&summary) % self.palette.len();
        if let Some(entry) = self.palette[slot]
            && entry.revision == self.revision
            && self.summaries[entry.index as usize] == summary
        {
            return Ok(entry.index);
        }
        let index = u32::try_from(self.summaries.len()).map_err(|_| Error::Capacity)?;
        self.summaries.try_reserve(1)?;
        self.summaries.push(summary);
        self.palette[slot] = Some(PaletteSlot {
            revision: self.revision,
            index,
        });
        Ok(index)
    }

    fn summary_view<T: Source + ?Sized>(
        &mut self,
        source: &T,
        view: View,
        classify: &impl Fn(u32) -> Sample,
    ) -> Result<Summary, Error<T::Error>> {
        match view.cell {
            Cell::Uniform(value) => Ok(Summary::uniform_value(value, view.lod, classify(value))),
            Cell::Node(index) => {
                if view.lod == 0 {
                    return Err(Error::Depth);
                }
                let level = (view.lod - 1) / 2;
                let position = if view.group == WHOLE {
                    8
                } else {
                    view.group as usize
                };
                let key = Key {
                    identity: source.identity(index, level).map_err(Error::Read)?,
                    index,
                    level,
                };
                if let Some(values) = self.cached(key) {
                    return Ok(values[position]);
                }
                Ok(self.ensure(source, key, classify)?[position])
            }
        }
    }

    fn ensure<T: Source + ?Sized>(
        &mut self,
        source: &T,
        key: Key,
        classify: &impl Fn(u32) -> Sample,
    ) -> Result<[Summary; 9], Error<T::Error>> {
        self.frames.clear();
        self.push_frame(source, key)?;
        loop {
            let frame = self.frames.last().expect("归约帧非空");
            if frame.next == 64 {
                let frame = self.frames.pop().expect("已检查帧");
                let mut values = [Summary::EMPTY; 9];
                for group in 0..8 {
                    let children = std::array::from_fn(|slot| frame.values[group * 8 + slot]);
                    values[group] = Summary::combine(children);
                }
                values[8] = Summary::combine(std::array::from_fn(|slot| values[slot]));
                if !self.cache.is_empty() {
                    let slot = hash(&frame.key) % self.cache.len();
                    if self.cache[slot].is_none() {
                        self.cache_len += 1;
                    }
                    self.cache[slot] = Some(Cached {
                        key: frame.key,
                        values,
                    });
                }
                if let Some(parent) = self.frames.last_mut() {
                    parent.values[parent.next] = values[8];
                    parent.next += 1;
                } else {
                    return Ok(values);
                }
            } else {
                let cell = frame.cells[frame.next];
                let level = frame.key.level;
                match cell {
                    Cell::Uniform(value) => {
                        let summary = Summary::uniform_value(value, level * 2, classify(value));
                        let frame = self.frames.last_mut().expect("帧仍有效");
                        frame.values[frame.next] = summary;
                        frame.next += 1;
                    }
                    Cell::Node(index) => {
                        if level == 0 {
                            return Err(Error::Depth);
                        }
                        let level = level - 1;
                        let key = Key {
                            identity: source.identity(index, level).map_err(Error::Read)?,
                            index,
                            level,
                        };
                        if let Some(values) = self.cached(key) {
                            let value = values[8];
                            let frame = self.frames.last_mut().expect("帧仍有效");
                            frame.values[frame.next] = value;
                            frame.next += 1;
                        } else {
                            self.push_frame(source, key)?;
                        }
                    }
                }
            }
        }
    }

    fn push_frame<T: Source + ?Sized>(
        &mut self,
        source: &T,
        key: Key,
    ) -> Result<(), Error<T::Error>> {
        let node = source.node(key.index, key.level).map_err(Error::Read)?;
        let cells = std::array::from_fn(|slot| node.child(slot as u8));
        self.frames.try_reserve(1)?;
        self.frames.push(Frame {
            key,
            cells,
            values: [Summary::EMPTY; 64],
            next: 0,
        });
        Ok(())
    }

    fn cached(&self, key: Key) -> Option<&[Summary; 9]> {
        if self.cache.is_empty() {
            return None;
        }
        self.cache[hash(&key) % self.cache.len()]
            .as_ref()
            .filter(|entry| entry.key == key)
            .map(|entry| &entry.values)
    }

    fn push_build<T: Source + ?Sized>(
        &mut self,
        source: &T,
        view: View,
        level: u8,
    ) -> Result<(), Error<T::Error>> {
        let mut children = [view; 64];
        if let Cell::Node(index) = view.cell
            && view.group == WHOLE
        {
            let node = source
                .node(index, (view.lod - 1) / 2)
                .map_err(Error::Read)?;
            children = std::array::from_fn(|slot| View {
                cell: node.child(slot as u8),
                lod: view.lod - 2,
                group: WHOLE,
            });
        } else {
            let halves = split(source, view)?;
            for group in 0..8 {
                children[group * 8..group * 8 + 8].copy_from_slice(&split(source, halves[group])?);
            }
        }
        self.build_frames.try_reserve(1)?;
        self.build_frames.push(BuildFrame {
            key: build_key(source, view, level)?,
            children,
            values: [Cell::Uniform(0); 64],
            next: 0,
            level,
        });
        Ok(())
    }
}

fn build_key<T: Source + ?Sized>(
    source: &T,
    view: View,
    level: u8,
) -> Result<BuildKey, Error<T::Error>> {
    let Cell::Node(index) = view.cell else {
        unreachable!("均匀区域不建立构建帧")
    };
    let source_level = (view.lod - 1) / 2;
    Ok(BuildKey {
        source: Key {
            identity: source.identity(index, source_level).map_err(Error::Read)?,
            index,
            level: source_level,
        },
        group: view.group,
        level,
    })
}

fn hash(value: &impl Hash) -> usize {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish() as usize
}

fn validate<E>(region: Region) -> Result<(), Error<E>> {
    if region.root_lod > 42
        || region.lod > region.root_lod
        || (matches!(region.root, Cell::Node(_))
            && (region.root_lod == 0 || region.root_lod % 2 != 0))
    {
        return Err(Error::Region);
    }
    let extent = 1_u64 << region.lod;
    let size = 1_u64 << region.root_lod;
    if region
        .minimum
        .into_iter()
        .any(|minimum| minimum % extent != 0 || minimum > size - extent)
    {
        return Err(Error::Region);
    }
    Ok(())
}

fn locate<T: Tree + ?Sized>(source: &T, region: Region) -> Result<View, Error<T::Error>> {
    let mut view = View {
        cell: region.root,
        lod: region.root_lod,
        group: WHOLE,
    };
    while view.lod > region.lod {
        let bit = view.lod - 1;
        let slot = ((region.minimum[0] >> bit) & 1)
            | (((region.minimum[1] >> bit) & 1) << 1)
            | (((region.minimum[2] >> bit) & 1) << 2);
        view = split(source, view)?[slot as usize];
    }
    Ok(view)
}

fn split<T: Tree + ?Sized>(source: &T, view: View) -> Result<[View; 8], Error<T::Error>> {
    if view.lod == 0 {
        return Err(Error::Depth);
    }
    let lod = view.lod - 1;
    match view.cell {
        Cell::Uniform(_) => Ok([View {
            lod,
            group: WHOLE,
            ..view
        }; 8]),
        Cell::Node(_) if view.group == WHOLE => Ok(std::array::from_fn(|slot| View {
            cell: view.cell,
            lod,
            group: slot as u8,
        })),
        Cell::Node(index) => {
            let node = source
                .node(index, (view.lod - 1) / 2)
                .map_err(Error::Read)?;
            Ok(std::array::from_fn(|slot| View {
                cell: node.child(view.group * 8 + slot as u8),
                lod,
                group: WHOLE,
            }))
        }
    }
}

fn emit_node<E>(output: &mut LodTree, level: u8, children: [Cell; 64]) -> Result<Cell, Error<E>> {
    if matches!(children[0], Cell::Uniform(_)) && children.iter().all(|&cell| cell == children[0]) {
        return Ok(children[0]);
    }
    let mut values = [0; 64];
    let mut pointers = 0_u64;
    for (slot, cell) in children.into_iter().enumerate() {
        match cell {
            Cell::Uniform(value) => values[slot] = value,
            Cell::Node(index) => {
                values[slot] = index;
                pointers |= 1 << slot;
            }
        }
    }
    let nodes = &mut output.nodes[level as usize];
    let index = u32::try_from(nodes.len()).map_err(|_| Error::Capacity)?;
    nodes.try_reserve(1)?;
    nodes.push(OwnedNode { values, pointers });
    Ok(Cell::Node(index))
}

#[cfg(test)]
#[path = "lod_tests.rs"]
mod tests;
