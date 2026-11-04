//! 借用 64 叉树直接提取层级界面，再对每个有向切面作最少矩形分解。
//!
//! 层级面相交路径以八分视图借用每个 64 叉节点，
//! 显式栈遍历中心块的内部界面和六个邻居边界；均匀区域直接产生二进制正方形，
//! 不解码 64³ 材料。树、任务和稀疏面缓存采用分离数组。
//! 树适配、材料含义和线程调度归调用方；一次任务独占 scratch 和输出。
//!
//! 输出为 [Quad]。每个方向、切面和标签分别分解；最少矩形不跨方向、切面或标签。
//! 完整 u32 标签只在单个切面内映射为矩形内核的非零短标签，零标签也可输出。

use std::num::NonZeroU16;

use rectangle_decomposition::optimal::{
    PackedRectangles64, QuadLeaf64, SparseOptimalScratch64, SparseQuadError,
};

use crate::{Direction, Quad};

/// 均匀区域或下一层节点；节点编号仅在其所属树和层内解释。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Cell {
    /// 当前区域每个体素具有相同的完整 u32 值。
    Uniform(u32),
    /// 64 个 Morton 顺序子区域的节点编号。
    Node(u32),
}

/// 一个节点的借用视图；payload 与指针位图分离，不复制完整节点。
#[derive(Clone, Copy)]
pub struct Node<'a> {
    values: &'a [u32; 64],
    pointers: u64,
}

impl<'a> Node<'a> {
    /// 借用 64 个局部 Morton 子项；位图置位表示对应值是下一层节点编号。
    ///
    /// 每轴两位按 x/y/z 交错。零位表示均匀值，所有 u32 均合法；位图不是占据掩码。
    pub const fn new(values: &'a [u32; 64], pointers: u64) -> Self {
        Self { values, pointers }
    }

    /// 读取一个 Morton 子项；置位指针仍由所属树解释。
    ///
    /// # Panics
    ///
    /// `slot >= 64` 时 panic。
    pub const fn child(self, slot: u8) -> Cell {
        let value = self.values[slot as usize];
        if self.pointers & (1_u64 << slot) == 0 {
            Cell::Uniform(value)
        } else {
            Cell::Node(value)
        }
    }
}

/// 只读分层树；实例和 root 的生命周期由调用方维护。
pub trait Tree {
    /// 存储读取失败的原始错误。
    type Error;

    /// 借用层内节点；level 为 2、1、0，分别覆盖 64³、16³、4³ 区域。
    ///
    /// 编号失效或节点缺失应返回错误；借用期间内容不得改变。level 为零的子项须均匀，
    /// 内核检查实际访问到的过深指针并返回 [Error::Depth]，不会递归或继续读取。
    fn node(&self, index: u32, level: u8) -> Result<Node<'_>, Self::Error>;
}

/// 64³ 局部树的只读入口；六个邻居各使用自身局部坐标。
pub struct Block<'a, T: Tree + ?Sized> {
    tree: &'a T,
    root: Cell,
}

impl<T: Tree + ?Sized> Copy for Block<'_, T> {}

impl<T: Tree + ?Sized> Clone for Block<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, T: Tree + ?Sized> Block<'a, T> {
    /// 借用树和覆盖完整 64³ 的 root；root 已由应用定位，不在内核中查找世界坐标。
    pub const fn new(tree: &'a T, root: Cell) -> Self {
        Self { tree, root }
    }
}

/// 树读取、深度、分配或矩形分解失败；失败后输出可能不完整，须重试整块。
#[derive(Debug, thiserror::Error)]
pub enum Error<E> {
    /// 保留树适配器的底层来源。
    #[error("稀疏树读取失败")]
    Read(#[source] E),
    /// 体素层仍出现内部指针，输入树不满足固定深度。
    #[error("稀疏树指针超过64³深度")]
    Depth,
    /// 任务、映射或输出缓冲无法申请容量。
    #[error("稀疏表面分配失败")]
    Allocation(#[from] std::collections::TryReserveError),
    /// 保留最少矩形内核的输入或容量错误。
    #[error("稀疏矩形分解失败")]
    Rectangles(#[from] SparseQuadError),
}

#[derive(Default)]
struct Trees {
    kinds: Vec<u8>,
    values: Vec<u32>,
    groups: Vec<u8>,
    sources: Vec<u8>,
    children: Vec<usize>,
    child_bases: Vec<usize>,
}

impl Trees {
    fn clear(&mut self) {
        self.kinds.clear();
        self.values.clear();
        self.groups.clear();
        self.sources.clear();
        self.children.clear();
        self.child_bases.clear();
    }

    fn push<E>(&mut self, cell: Cell, source: u8) -> Result<usize, Error<E>> {
        self.push_kind(cell, source, 0)
    }

    fn push_kind<E>(&mut self, cell: Cell, source: u8, group: u8) -> Result<usize, Error<E>> {
        self.kinds.try_reserve(1)?;
        self.values.try_reserve(1)?;
        self.groups.try_reserve(1)?;
        self.sources.try_reserve(1)?;
        self.child_bases.try_reserve(1)?;
        let index = self.values.len();
        let (kind, value) = match cell {
            Cell::Uniform(value) => (0, value),
            Cell::Node(index) => (if group == 0 { 1 } else { 2 }, index),
        };
        self.kinds.push(kind);
        self.values.push(value);
        self.groups.push(group);
        self.sources.push(source);
        self.child_bases.push(usize::MAX);
        Ok(index)
    }

    fn expand<T: Tree + ?Sized>(
        &mut self,
        index: usize,
        lod: u8,
        blocks: &[Option<Block<'_, T>>; 7],
    ) -> Result<[usize; 8], Error<T::Error>> {
        if lod == 0 {
            return Err(Error::Depth);
        }
        if self.kinds[index] == 0 {
            return Ok([index; 8]);
        }
        let base = self.child_bases[index];
        if base != usize::MAX {
            return Ok(std::array::from_fn(|slot| self.children[base + slot]));
        }
        let source = self.sources[index];
        let node = blocks[source as usize]
            .expect("缓存项总有来源")
            .tree
            .node(self.values[index], (lod - 1) / 2)
            .map_err(Error::Read)?;
        let mut out = [0; 8];
        for slot in 0..8_u8 {
            let (cell, group) = if self.kinds[index] == 1 {
                let first = node.child(slot * 8);
                if matches!(first, Cell::Uniform(_))
                    && (1..8).all(|offset| node.child(slot * 8 + offset) == first)
                {
                    (first, 0)
                } else {
                    (Cell::Node(self.values[index]), slot + 1)
                }
            } else {
                (node.child((self.groups[index] - 1) * 8 + slot), 0)
            };
            if lod == 1 && matches!(cell, Cell::Node(_)) {
                return Err(Error::Depth);
            }
            out[slot as usize] = self.push_kind(cell, source, group)?;
        }
        self.children.try_reserve(8)?;
        let base = self.children.len();
        self.children.extend_from_slice(&out);
        self.child_bases[index] = base;
        Ok(out)
    }

    fn bytes(&self) -> usize {
        self.kinds.capacity()
            + self.values.capacity() * 4
            + self.groups.capacity()
            + self.sources.capacity()
            + (self.children.capacity() + self.child_bases.capacity()) * size_of::<usize>()
    }
}

#[derive(Clone, Copy)]
struct Cube {
    tree: usize,
    origin: [u8; 3],
    lod: u8,
}

#[derive(Default)]
struct Cubes {
    trees: Vec<usize>,
    origins: Vec<[u8; 3]>,
    lods: Vec<u8>,
}

impl Cubes {
    fn clear(&mut self) {
        self.trees.clear();
        self.origins.clear();
        self.lods.clear();
    }

    fn push<E>(&mut self, cube: Cube) -> Result<(), Error<E>> {
        self.trees.try_reserve(1)?;
        self.origins.try_reserve(1)?;
        self.lods.try_reserve(1)?;
        self.trees.push(cube.tree);
        self.origins.push(cube.origin);
        self.lods.push(cube.lod);
        Ok(())
    }

    fn pop(&mut self) -> Option<Cube> {
        Some(Cube {
            tree: self.trees.pop()?,
            origin: self.origins.pop().expect("任务列等长"),
            lod: self.lods.pop().expect("任务列等长"),
        })
    }

    fn bytes(&self) -> usize {
        self.trees.capacity() * size_of::<usize>()
            + self.origins.capacity() * 3
            + self.lods.capacity()
    }
}

#[derive(Clone, Copy)]
struct Face {
    negative: usize,
    positive: usize,
    origin: [u8; 3],
    lod: u8,
    axis: u8,
    plane: u8,
    owners: u8,
}

#[derive(Default)]
struct Faces {
    negative: Vec<usize>,
    positive: Vec<usize>,
    origins: Vec<[u8; 3]>,
    lods: Vec<u8>,
    axes: Vec<u8>,
    planes: Vec<u8>,
    owners: Vec<u8>,
}

impl Faces {
    fn clear(&mut self) {
        self.negative.clear();
        self.positive.clear();
        self.origins.clear();
        self.lods.clear();
        self.axes.clear();
        self.planes.clear();
        self.owners.clear();
    }

    fn push<E>(&mut self, face: Face) -> Result<(), Error<E>> {
        self.negative.try_reserve(1)?;
        self.positive.try_reserve(1)?;
        self.origins.try_reserve(1)?;
        self.lods.try_reserve(1)?;
        self.axes.try_reserve(1)?;
        self.planes.try_reserve(1)?;
        self.owners.try_reserve(1)?;
        self.negative.push(face.negative);
        self.positive.push(face.positive);
        self.origins.push(face.origin);
        self.lods.push(face.lod);
        self.axes.push(face.axis);
        self.planes.push(face.plane);
        self.owners.push(face.owners);
        Ok(())
    }

    fn pop(&mut self) -> Option<Face> {
        Some(Face {
            negative: self.negative.pop()?,
            positive: self.positive.pop().expect("任务列等长"),
            origin: self.origins.pop().expect("任务列等长"),
            lod: self.lods.pop().expect("任务列等长"),
            axis: self.axes.pop().expect("任务列等长"),
            plane: self.planes.pop().expect("任务列等长"),
            owners: self.owners.pop().expect("任务列等长"),
        })
    }

    fn bytes(&self) -> usize {
        (self.negative.capacity() + self.positive.capacity()) * size_of::<usize>()
            + self.origins.capacity() * 3
            + self.lods.capacity()
            + self.axes.capacity()
            + self.planes.capacity()
            + self.owners.capacity()
    }
}

#[derive(Default)]
struct Leaves {
    directions: Vec<u8>,
    planes: Vec<u8>,
    origins: Vec<[u8; 2]>,
    lods: Vec<u8>,
    materials: Vec<u32>,
}

impl Leaves {
    fn clear(&mut self) {
        self.directions.clear();
        self.planes.clear();
        self.origins.clear();
        self.lods.clear();
        self.materials.clear();
    }

    fn push<E>(&mut self, face: Face, positive: bool, label: u32) -> Result<(), Error<E>> {
        self.directions.try_reserve(1)?;
        self.planes.try_reserve(1)?;
        self.origins.try_reserve(1)?;
        self.lods.try_reserve(1)?;
        self.materials.try_reserve(1)?;
        self.directions.push(face.axis * 2 + u8::from(positive));
        self.planes.push(face.plane);
        self.origins.push([
            face.origin[(face.axis as usize + 1) % 3],
            face.origin[(face.axis as usize + 2) % 3],
        ]);
        self.lods.push(face.lod);
        self.materials.push(label);
        Ok(())
    }

    fn bytes(&self) -> usize {
        self.directions.capacity()
            + self.planes.capacity()
            + self.origins.capacity() * 2
            + self.lods.capacity()
            + self.materials.capacity() * 4
    }
}

/// 单任务独占缓存；均匀块不分配密集体素或面图，容量随历史复杂度保留。
///
/// 矩形 scratch 固定容量内联保存；建议在 worker 初始化时构造并复用，避免逐块创建。
/// 总工作集还须计入输入树、并发任务、输出及应用持有的新旧场景。
#[derive(Default)]
pub struct Scratch {
    trees: Trees,
    cubes: Cubes,
    faces: Faces,
    leaves: Leaves,
    order: Vec<usize>,
    offsets: Vec<usize>,
    labels: Vec<u32>,
    quad_leaves: Vec<QuadLeaf64>,
    rectangles: SparseOptimalScratch64,
    packed: PackedRectangles64,
}

impl Scratch {
    /// 自身和内部 Vec 保留容量的总字节数，不含输入树或输出 Vec。
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.trees.bytes()
            + self.cubes.bytes()
            + self.faces.bytes()
            + self.leaves.bytes()
            + self.order.capacity() * size_of::<usize>()
            + self.offsets.capacity() * size_of::<usize>()
            + self.labels.capacity() * 4
            + self.quad_leaves.capacity() * size_of::<QuadLeaf64>()
            + self
                .packed
                .retained_bytes()
                .saturating_sub(size_of::<PackedRectangles64>())
    }

    fn clear(&mut self) {
        self.trees.clear();
        self.cubes.clear();
        self.faces.clear();
        self.leaves.clear();
        self.labels.clear();
        self.quad_leaves.clear();
    }
}

/// 从中心树和六个相邻块提取不透明表面；缺失邻居视为零，非零值互相遮蔽。
///
/// 邻居顺序为 -X、+X、-Y、+Y、-Z、+Z；仅输出中心块所有的面。输出先清空，
/// 完成后是整块替换结果；读取、深度、分配和分解失败保留底层错误，输出可能不完整。
pub fn mesh<T: Tree + ?Sized>(
    center: Block<'_, T>,
    neighbors: [Option<Block<'_, T>>; 6],
    scratch: &mut Scratch,
    output: &mut Vec<Quad>,
) -> Result<(), Error<T::Error>> {
    mesh_by(center, neighbors, scratch, output, |source, neighbor| {
        (source != 0 && neighbor == 0).then_some(source)
    })
}

/// 按应用的相邻值策略提取表面；包括零材料、相同材料对和零输出标签。
///
/// face(source, neighbor) 必须对相同参数返回相同结果；调用次数和遍历顺序不构成契约，
/// 均匀区域仅求值一次即可覆盖多个面。Some(label) 生成面，None 遮蔽；两个相反方向
/// 可独立可见。所有权、邻居顺序和失败后的替换规则同 [mesh]。
pub fn mesh_by<T: Tree + ?Sized>(
    center: Block<'_, T>,
    neighbors: [Option<Block<'_, T>>; 6],
    scratch: &mut Scratch,
    output: &mut Vec<Quad>,
    mut face: impl FnMut(u32, u32) -> Option<u32>,
) -> Result<(), Error<T::Error>> {
    mesh_by_direction(center, neighbors, scratch, output, |a, b, _| face(a, b))
}

/// 按源值、邻值与外法线选择可见面及合并标签。
///
/// 策略须对相同三元组返回相同结果；其余所有权和失败规则同 [`mesh_by`]。
pub fn mesh_by_direction<T: Tree + ?Sized>(
    center: Block<'_, T>,
    neighbors: [Option<Block<'_, T>>; 6],
    scratch: &mut Scratch,
    output: &mut Vec<Quad>,
    face: impl FnMut(u32, u32, Direction) -> Option<u32>,
) -> Result<(), Error<T::Error>> {
    mesh_by_direction_boundaries(center, neighbors, [true; 6], scratch, output, face)
}

/// 仅提取指定外边界及全部内部界面；`boundaries` 顺序同 [`Direction::ALL`]。
///
/// 禁用的外边界完全跳过，供上层以混合尺度共享界面替换；不会生成裙边。
pub fn mesh_by_direction_boundaries<T: Tree + ?Sized>(
    center: Block<'_, T>,
    neighbors: [Option<Block<'_, T>>; 6],
    boundaries: [bool; 6],
    scratch: &mut Scratch,
    output: &mut Vec<Quad>,
    mut face: impl FnMut(u32, u32, Direction) -> Option<u32>,
) -> Result<(), Error<T::Error>> {
    output.clear();
    scratch.clear();
    let blocks = [
        Some(center),
        neighbors[0],
        neighbors[1],
        neighbors[2],
        neighbors[3],
        neighbors[4],
        neighbors[5],
    ];
    let root = scratch.trees.push(center.root, 0)?;
    scratch.cubes.push(Cube {
        tree: root,
        origin: [0; 3],
        lod: 6,
    })?;
    for direction in Direction::ALL {
        if !boundaries[direction as usize] {
            continue;
        }
        let source = direction as usize + 1;
        let neighbor = scratch.trees.push(
            blocks[source].map_or(Cell::Uniform(0), |block| block.root),
            source as u8,
        )?;
        let positive = direction.positive();
        emit_or_push(
            scratch,
            Face {
                negative: if positive { root } else { neighbor },
                positive: if positive { neighbor } else { root },
                origin: [0; 3],
                lod: 6,
                axis: direction.axis() as u8,
                plane: if positive { 64 } else { 0 },
                owners: if positive { 1 } else { 2 },
            },
            &mut face,
        )?;
    }
    while let Some(cube) = scratch.cubes.pop() {
        if scratch.trees.kinds[cube.tree] == 0 {
            if cube.lod != 0 {
                for axis in 0..3_u8 {
                    let value = scratch.trees.values[cube.tree];
                    let positive_label = face(value, value, direction(axis, true));
                    let negative_label = face(value, value, direction(axis, false));
                    for offset in 1..(1 << cube.lod) {
                        let task = Face {
                            negative: cube.tree,
                            positive: cube.tree,
                            origin: cube.origin,
                            lod: cube.lod,
                            axis,
                            plane: cube.origin[axis as usize] + offset,
                            owners: 3,
                        };
                        if let Some(label) = positive_label {
                            scratch.leaves.push(task, true, label)?;
                        }
                        if let Some(label) = negative_label {
                            scratch.leaves.push(task, false, label)?;
                        }
                    }
                }
            }
            continue;
        }
        let children = scratch.trees.expand(cube.tree, cube.lod, &blocks)?;
        let lod = cube.lod - 1;
        for slot in 0..8_u8 {
            scratch.cubes.push(Cube {
                tree: children[slot as usize],
                origin: child_origin(cube.origin, slot, 1 << lod),
                lod,
            })?;
        }
        for axis in 0..3_u8 {
            for u in 0..2_u8 {
                for v in 0..2_u8 {
                    let low = face_slot(axis, false, u, v);
                    let high = face_slot(axis, true, u, v);
                    emit_or_push(
                        scratch,
                        Face {
                            negative: children[low as usize],
                            positive: children[high as usize],
                            origin: child_origin(cube.origin, low, 1 << lod),
                            lod,
                            axis,
                            plane: cube.origin[axis as usize] + (1 << lod),
                            owners: 3,
                        },
                        &mut face,
                    )?;
                }
            }
        }
    }
    while let Some(task) = scratch.faces.pop() {
        if scratch.trees.kinds[task.negative] == 0 && scratch.trees.kinds[task.positive] == 0 {
            let negative = scratch.trees.values[task.negative];
            let positive = scratch.trees.values[task.positive];
            if task.owners & 1 != 0
                && let Some(label) = face(negative, positive, direction(task.axis, true))
            {
                scratch.leaves.push(task, true, label)?;
            }
            if task.owners & 2 != 0
                && let Some(label) = face(positive, negative, direction(task.axis, false))
            {
                scratch.leaves.push(task, false, label)?;
            }
            continue;
        }
        let negative = scratch.trees.expand(task.negative, task.lod, &blocks)?;
        let positive = scratch.trees.expand(task.positive, task.lod, &blocks)?;
        let lod = task.lod - 1;
        for u in 0..2_u8 {
            for v in 0..2_u8 {
                let low = face_slot(task.axis, false, u, v);
                let high = face_slot(task.axis, true, u, v);
                emit_or_push(
                    scratch,
                    Face {
                        negative: negative[high as usize],
                        positive: positive[low as usize],
                        origin: child_origin(task.origin, low, 1 << lod),
                        lod,
                        ..task
                    },
                    &mut face,
                )?;
            }
        }
    }
    decompose(scratch, output)
}

fn emit_or_push<E>(
    scratch: &mut Scratch,
    task: Face,
    face: &mut impl FnMut(u32, u32, Direction) -> Option<u32>,
) -> Result<(), Error<E>> {
    if scratch.trees.kinds[task.negative] == 0 && scratch.trees.kinds[task.positive] == 0 {
        let negative = scratch.trees.values[task.negative];
        let positive = scratch.trees.values[task.positive];
        if task.owners & 1 != 0
            && let Some(label) = face(negative, positive, direction(task.axis, true))
        {
            scratch.leaves.push(task, true, label)?;
        }
        if task.owners & 2 != 0
            && let Some(label) = face(positive, negative, direction(task.axis, false))
        {
            scratch.leaves.push(task, false, label)?;
        }
        Ok(())
    } else {
        scratch.faces.push(task)
    }
}

const fn direction(axis: u8, positive: bool) -> Direction {
    Direction::ALL[axis as usize * 2 + positive as usize]
}

fn decompose<E>(scratch: &mut Scratch, output: &mut Vec<Quad>) -> Result<(), Error<E>> {
    const BINS: usize = 3 * 65 * 2;
    scratch
        .offsets
        .try_reserve((BINS + 1).saturating_sub(scratch.offsets.len()))?;
    scratch.offsets.resize(BINS + 1, 0);
    scratch.offsets.fill(0);
    let mut labels = [0; BINS];
    let mut uniform = [true; BINS];
    for index in 0..scratch.leaves.materials.len() {
        let bin = leaf_bin(&scratch.leaves, index);
        if scratch.offsets[bin + 1] == 0 {
            labels[bin] = scratch.leaves.materials[index];
        } else if labels[bin] != scratch.leaves.materials[index] {
            uniform[bin] = false;
        }
        scratch.offsets[bin + 1] += 1;
    }
    for bin in 0..BINS {
        scratch.offsets[bin + 1] += scratch.offsets[bin];
    }
    let count = scratch.leaves.materials.len();
    scratch
        .order
        .try_reserve(count.saturating_sub(scratch.order.len()))?;
    // 所有槽均由散布覆盖；相同长度复用旧初始化，避免逐帧先清零完整索引数组。
    scratch.order.resize(count, 0);
    let mut cursors = [0; BINS];
    cursors.copy_from_slice(&scratch.offsets[..BINS]);
    for index in 0..count {
        let bin = leaf_bin(&scratch.leaves, index);
        scratch.order[cursors[bin]] = index;
        cursors[bin] += 1;
    }
    for bin in 0..BINS {
        let start = scratch.offsets[bin];
        let end = scratch.offsets[bin + 1];
        if start == end {
            continue;
        }
        let direction = bin / 130 * 2 + bin % 2;
        let plane = bin % 130 / 2;
        scratch.labels.clear();
        if uniform[bin] {
            scratch.labels.try_reserve(1)?;
            scratch.labels.push(labels[bin]);
        } else {
            scratch.labels.try_reserve(end - start)?;
            scratch.labels.extend(
                scratch.order[start..end]
                    .iter()
                    .map(|&index| scratch.leaves.materials[index]),
            );
            scratch.labels.sort_unstable();
            scratch.labels.dedup();
        }
        scratch.quad_leaves.clear();
        scratch.quad_leaves.try_reserve(end - start)?;
        for &index in &scratch.order[start..end] {
            let label = if uniform[bin] {
                1
            } else {
                scratch
                    .labels
                    .binary_search(&scratch.leaves.materials[index])
                    .expect("标签已收集")
                    + 1
            };
            let value = NonZeroU16::new(label as u16).expect("单切面至多4096标签");
            let [u, v] = scratch.leaves.origins[index];
            scratch.quad_leaves.push(QuadLeaf64 {
                u,
                v,
                lod: scratch.leaves.lods[index],
                value,
            });
        }
        scratch
            .rectangles
            .decompose_packed(&scratch.quad_leaves, &mut scratch.packed)?;
        output.try_reserve(scratch.packed.len())?;
        let axis = direction / 2;
        for rectangle in scratch.packed.iter() {
            let mut origin = [0; 3];
            origin[axis] = plane as u32;
            origin[(axis + 1) % 3] = rectangle.x.start as u32;
            origin[(axis + 2) % 3] = rectangle.y.start as u32;
            output.push(Quad {
                origin,
                direction: Direction::ALL[direction],
                width: (rectangle.x.end - rectangle.x.start) as u32,
                height: (rectangle.y.end - rectangle.y.start) as u32,
                material: scratch.labels[rectangle.value as usize - 1],
            });
        }
    }
    Ok(())
}

fn leaf_bin(leaves: &Leaves, index: usize) -> usize {
    let direction = leaves.directions[index] as usize;
    direction / 2 * 130 + leaves.planes[index] as usize * 2 + direction % 2
}

#[cfg(test)]
mod tests;

const fn face_slot(axis: u8, positive: bool, u: u8, v: u8) -> u8 {
    (positive as u8) << axis | u << ((axis + 1) % 3) | v << ((axis + 2) % 3)
}

const fn child_origin(origin: [u8; 3], slot: u8, step: u8) -> [u8; 3] {
    [
        origin[0] + step * (slot & 1),
        origin[1] + step * ((slot >> 1) & 1),
        origin[2] + step * ((slot >> 2) & 1),
    ]
}
