//! 稳定数据槽、活动集合与稀疏反查的单一CPU所有者；准备不修改已发布表。

use crate as light_sampling;
use std::collections::HashMap;

/// 活动光源表输入、事务或构建失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 身份、布局或阶段不符合目录协议。
    #[error("光源目录：{0}")]
    Invalid(&'static str),
    /// CPU 内存申请失败，保留来源。
    #[error(transparent)]
    Allocation(#[from] std::collections::TryReserveError),
}

const INITIAL_CAPACITY: usize = 16;
const META: u8 = 1;
const VERTICES: u8 = 2;
const REGION: u8 = 4;
const POWER: u8 = 8;

/// 四列的完整容量镜像；只在显式容量增长时产生。
pub struct Tables {
    /// 每槽面积协议与 quad 身份。
    pub meta: Vec<[u32; 4]>,
    /// 每槽三个 float4 的局部四角。
    pub vertices: Vec<[f32; 4]>,
    /// 每槽网格坐标与区块内灯序号、发射侧。
    pub regions: Vec<[u32; 4]>,
    /// quad 身份到稳定槽的线性探测表。
    pub inverse: Vec<[u32; 4]>,
}

/// 已校验的互不重叠16B行；顺序不影响发布，不要求按索引排序。
#[derive(Default)]
pub struct TablePatches {
    /// 面积协议与 quad 身份的变化行。
    pub meta: Vec<(usize, [u32; 4])>,
    /// 局部四角的变化行。
    pub vertices: Vec<(usize, [f32; 4])>,
    /// 反查哈希桶的变化行。
    pub inverse: Vec<(usize, [u32; 4])>,
    /// 网格位置、区块内灯序号与侧掩码的变化行。
    pub regions: Vec<(usize, [u32; 4])>,
}

#[derive(Clone, Copy, Default)]
/// 一盏灯的稳定数据；meta.w 是唯一 quad 身份，region.w 低两位为发射侧。
/// vertices 是 pack_vertices 的局部四角，power 是资源期确定的有限正权重。
pub struct Record {
    /// 每槽面积协议与 quad 身份。
    pub meta: [u32; 4],
    /// 每槽三个 float4 的局部四角。
    pub vertices: [[f32; 4]; 3],
    /// 网格坐标位模式。w 为区块内灯序号左移两位后与侧掩码合并。
    pub region: [u32; 4],
    /// 有限正的光源提案功率。
    pub power: f32,
}

struct Edit {
    record: Record,
    mask: u8,
}

#[derive(Default)]
/// 活动灯的单一所有者；槽删除可复用而不移动其他灯。
/// 发布稳定灯槽与稀疏反查；活动顺序只用于 CPU 槽回收。
pub struct Registry {
    capacity: usize,
    span: u32,
    free: Vec<u32>,
    active: Vec<u32>,
    meta: Vec<[u32; 4]>,
    vertices: Vec<[f32; 4]>,
    regions: Vec<[u32; 4]>,
    powers: Vec<f32>,
    inverse: Vec<[u32; 4]>,
    ranks: Vec<u32>,
}

/// 未发布的稀疏修改；仅适用于产生它的目录版本，丢弃不修改目录。
pub struct Delta {
    capacity: usize,
    span: u32,
    count: usize,
    free_used: usize,
    released: Vec<u32>,
    active: HashMap<usize, u32>,
    records: HashMap<u32, Edit>,
    additions: Vec<(u32, Record)>,
    added_active: Vec<u32>,
    added_start: usize,
    inverse: HashMap<usize, [u32; 4]>,
    ranks: HashMap<u32, usize>,
}

/// 借用当前目录收集修改；先删改，再批量新增，最后 finish。
pub struct Writer<'a> {
    current: &'a Registry,
    delta: Delta,
    removals: Vec<(u32, u32)>,
    insertions: Vec<(u32, u32)>,
}

fn allocation(error: std::collections::TryReserveError) -> Error {
    Error::Allocation(error)
}

/// 申请精确暂存容量，不初始化元素；失败保留分配来源。
pub fn reserved<T>(count: usize) -> Result<Vec<T>, Error> {
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(allocation)?;
    Ok(values)
}

fn pushed<T>(values: &mut Vec<T>, value: T) -> Result<(), Error> {
    values.try_reserve(1).map_err(allocation)?;
    values.push(value);
    Ok(())
}

fn mapped<K: std::hash::Hash + Eq, V>(
    map: &mut HashMap<K, V>,
    key: K,
    value: V,
) -> Result<(), Error> {
    if !map.contains_key(&key) {
        map.try_reserve(1).map_err(allocation)?;
    }
    map.insert(key, value);
    Ok(())
}

impl Registry {
    /// 已发布data span之外按零记录读取，不初始化未使用的CPU尾部。
    pub fn record(&self, id: u32) -> Record {
        let index = id as usize;
        Record {
            meta: self.meta.get(index).copied().unwrap_or_default(),
            vertices: std::array::from_fn(|i| {
                self.vertices
                    .get(index * 3 + i)
                    .copied()
                    .unwrap_or_default()
            }),
            region: self.regions.get(index).copied().unwrap_or_default(),
            power: self.powers.get(index).copied().unwrap_or_default(),
        }
    }

    /// 校验完整表并预留提交内存；失败只改变保留容量，不发布记录。
    /// delta 与 tables 必须来自本目录当前版本的同一次 finish。
    pub fn reserve(&mut self, delta: &Delta, tables: Option<&Tables>) -> Result<(), Error> {
        self.ranks
            .try_reserve_exact(delta.capacity.saturating_sub(self.ranks.len()))
            .map_err(allocation)?;
        self.powers
            .try_reserve_exact(delta.capacity.saturating_sub(self.powers.len()))
            .map_err(allocation)?;
        if delta.capacity != self.capacity {
            let tables = tables.ok_or(Error::Invalid("发光增长阶段缺少完整表"))?;
            if tables.meta.len() != delta.capacity
                || tables.vertices.len() != delta.capacity * 3
                || tables.regions.len() != delta.capacity
                || tables.inverse.len() != delta.capacity * 2 + 1
                || tables.inverse[0][0] as usize != delta.count
            {
                return Err(Error::Invalid("发光完整表与准备容量不匹配"));
            }
        } else if tables.is_some() {
            return Err(Error::Invalid("发光未增长阶段包含多余完整表"));
        }
        if !self.moves_active(delta) {
            // 活动序列与GPU data容量一起保留，逐帧新增不会反复精确重分配。
            self.active
                .try_reserve_exact(delta.capacity - self.active.len())
                .map_err(allocation)?;
        }
        let free_len = self.free.len() - delta.free_used + delta.released.len();
        self.free
            .try_reserve(free_len.saturating_sub(self.free.len()))
            .map_err(allocation)?;
        Ok(())
    }

    fn moves_active(&self, delta: &Delta) -> bool {
        delta.added_start == 0
            && delta.added_active.len() == delta.count
            && delta.added_active.capacity() >= delta.capacity
            && self.active.capacity() < delta.capacity
    }

    /// GPU 上传成功后发布同一次 reserve 已校验的候选，不分配。
    /// 调用方须独占目录并保证 reserve 后没有其他发布，tables 不得被修改。
    pub fn commit(&mut self, delta: Delta, tables: Option<Tables>) {
        self.powers.resize(delta.span as usize, 0.);
        for (&id, edit) in &delta.records {
            if edit.mask & POWER != 0 {
                self.powers[id as usize] = edit.record.power;
            }
        }
        for &(id, record) in &delta.additions {
            self.powers[id as usize] = record.power;
        }
        self.ranks.resize(delta.span as usize, 0);
        for (&id, &rank) in &delta.ranks {
            self.ranks[id as usize] = rank as u32;
        }
        for (rank, &id) in (delta.added_start..).zip(&delta.added_active) {
            self.ranks[id as usize] = rank as u32;
        }
        if self.moves_active(&delta) {
            self.active = delta.added_active;
        } else {
            self.active.resize(delta.count, 0);
            for (rank, id) in delta.active {
                if rank < self.active.len() {
                    self.active[rank] = id;
                }
            }
            if !delta.added_active.is_empty() {
                self.active[delta.added_start..].copy_from_slice(&delta.added_active);
            }
        }
        if let Some(tables) = tables {
            // GPU已经从同一份wire复制；直接移交其存储，避免第二份完整SoA和inverse副本。
            self.meta = tables.meta;
            self.vertices = tables.vertices;
            self.regions = tables.regions;
            self.inverse = tables.inverse;
            self.meta.truncate(delta.span as usize);
            self.vertices.truncate(delta.span as usize * 3);
            self.regions.truncate(delta.span as usize);
        } else {
            // 之前移交的完整表保留p2容量；这里只扩展实际span，不触发分配。
            self.meta.resize(delta.span as usize, [0; 4]);
            self.vertices.resize(delta.span as usize * 3, [0.; 4]);
            self.regions.resize(delta.span as usize, [0; 4]);
            for (id, edit) in delta.records {
                let index = id as usize;
                if edit.mask & META != 0 {
                    self.meta[index] = edit.record.meta;
                }
                if edit.mask & VERTICES != 0 {
                    self.vertices[index * 3..index * 3 + 3].copy_from_slice(&edit.record.vertices);
                }
                if edit.mask & REGION != 0 {
                    self.regions[index] = edit.record.region;
                }
            }
            for (id, record) in delta.additions {
                let index = id as usize;
                self.meta[index] = record.meta;
                self.vertices[index * 3..index * 3 + 3].copy_from_slice(&record.vertices);
                self.regions[index] = record.region;
            }
            for (index, row) in delta.inverse {
                self.inverse[index] = row;
            }
        }
        self.free.truncate(self.free.len() - delta.free_used);
        self.free.extend(delta.released);
        self.span = delta.span;
        self.capacity = delta.capacity;
    }
}

impl<'a> Writer<'a> {
    /// 开始只读准备；返回后旧目录保持可用。
    pub fn new(current: &'a Registry) -> Self {
        Self {
            current,
            delta: Delta {
                capacity: current.capacity,
                span: current.span,
                count: current.active.len(),
                free_used: 0,
                released: Vec::new(),
                active: HashMap::new(),
                records: HashMap::new(),
                additions: Vec::new(),
                added_active: Vec::new(),
                added_start: current.active.len(),
                inverse: HashMap::new(),
                ranks: HashMap::new(),
            },
            removals: Vec::new(),
            insertions: Vec::new(),
        }
    }

    /// 新增阶段之前读取旧槽及其稀疏修改；新增之后只允许继续add并finish。
    pub fn record(&self, id: u32) -> Record {
        self.delta
            .records
            .get(&id)
            .map_or_else(|| self.current.record(id), |v| v.record)
    }

    fn rank(&self, id: u32) -> usize {
        self.delta
            .ranks
            .get(&id)
            .copied()
            .or_else(|| {
                self.current
                    .ranks
                    .get(id as usize)
                    .map(|&rank| rank as usize)
            })
            .unwrap_or(usize::MAX)
    }

    fn active(&self, rank: usize) -> u32 {
        if rank >= self.delta.count {
            return 0;
        }
        if rank >= self.delta.added_start
            && let Some(&id) = self.delta.added_active.get(rank - self.delta.added_start)
        {
            return id;
        }
        self.delta
            .active
            .get(&rank)
            .copied()
            .or_else(|| self.current.active.get(rank).copied())
            .unwrap_or(0)
    }

    fn edit(&mut self, id: u32, record: Record) -> Result<(), Error> {
        let old = self.current.record(id);
        let mask = (u8::from(old.meta != record.meta) * META)
            | (u8::from(
                old.vertices.map(|r| r.map(f32::to_bits))
                    != record.vertices.map(|r| r.map(f32::to_bits)),
            ) * VERTICES)
            | (u8::from(old.region != record.region) * REGION)
            | (u8::from(old.power.to_bits() != record.power.to_bits()) * POWER);
        if mask == 0 {
            self.delta.records.remove(&id);
            Ok(())
        } else {
            mapped(&mut self.delta.records, id, Edit { record, mask })
        }
    }

    /// 删除当前活动槽；必须先于新增，身份不属于活动集合时返回错误。
    pub fn remove(&mut self, id: u32) -> Result<(), Error> {
        if !self.delta.additions.is_empty() {
            return Err(Error::Invalid("发光删除必须先于批量新增"));
        }
        let record = self.record(id);
        let rank = self.rank(id);
        if rank >= self.delta.count || self.active(rank) != id {
            return Err(Error::Invalid("发光数据身份不属于当前活动集合"));
        }
        let last_rank = self.delta.count - 1;
        let last_id = self.active(last_rank);
        self.delta.count = last_rank;
        mapped(&mut self.delta.active, last_rank, 0)?;
        if rank != last_rank {
            mapped(&mut self.delta.active, rank, last_id)?;
            mapped(&mut self.delta.ranks, last_id, rank)?;
        }
        let key = record.meta[3]
            .checked_add(1)
            .ok_or(Error::Invalid("发光反查身份溢出"))?;
        pushed(&mut self.removals, (key, id))?;
        pushed(&mut self.delta.released, id)
    }

    /// 替换当前活动记录并保留活动序号；必须先于新增，非活动身份返回错误。
    pub fn replace(&mut self, id: u32, record: Record) -> Result<(), Error> {
        if !self.delta.additions.is_empty() {
            return Err(Error::Invalid("发光修改必须先于批量新增"));
        }
        let old = self.record(id);
        let rank = self.rank(id);
        if rank >= self.delta.count || self.active(rank) != id {
            return Err(Error::Invalid("发光数据身份不属于当前活动集合"));
        }
        if old.meta[3] != record.meta[3] {
            pushed(
                &mut self.removals,
                (
                    old.meta[3]
                        .checked_add(1)
                        .ok_or(Error::Invalid("发光反查身份溢出"))?,
                    id,
                ),
            )?;
            pushed(
                &mut self.insertions,
                (
                    record.meta[3]
                        .checked_add(1)
                        .ok_or(Error::Invalid("发光反查身份溢出"))?,
                    id,
                ),
            )?;
        }
        self.edit(id, record)
    }

    /// 复用空槽或追加记录，返回稳定槽号；容量或身份溢出返回错误。
    pub fn add(&mut self, record: Record) -> Result<u32, Error> {
        if self.delta.count >= light_sampling::SELECTORS as usize {
            return Err(Error::Invalid("活动发光quad数量超过24-bit均匀采样支持域"));
        }
        let id = if let Some(id) = self.delta.released.pop() {
            id
        } else if let Some(&id) = self.current.free.get(
            self.current
                .free
                .len()
                .wrapping_sub(self.delta.free_used + 1),
        ) {
            self.delta.free_used += 1;
            id
        } else {
            let id = self.delta.span;
            self.delta.span = id
                .checked_add(1)
                .ok_or(Error::Invalid("发光数据槽身份溢出"))?;
            id
        };
        if self.delta.added_active.is_empty() {
            self.delta.added_start = self.delta.count;
        }
        pushed(&mut self.delta.added_active, id)?;
        self.delta.count += 1;
        let key = record.meta[3]
            .checked_add(1)
            .ok_or(Error::Invalid("发光反查身份溢出"))?;
        if self.current.capacity != 0 {
            pushed(&mut self.insertions, (key, id))?;
        }
        // 新增已经进入最终批量阶段；连续记录无需再做每灯hash暂存/查询。
        if !self.delta.records.is_empty() {
            self.delta.records.remove(&id);
        }
        pushed(&mut self.delta.additions, (id, record))?;
        Ok(id)
    }

    /// 校验反查身份并准备目录写入。
    /// 容量增长返回完整表，其余返回稀疏行。GPU 准备成功后由目录所有者提交。
    /// # Errors
    /// 输入、容量或反查身份不合法时保留当前发布版本。
    pub fn finish_for_gpu(mut self) -> Result<(Delta, Option<Tables>, TablePatches), Error> {
        let needed = self.delta.span as usize;
        if needed > self.delta.capacity {
            self.delta.capacity = needed
                .max(INITIAL_CAPACITY)
                .checked_next_power_of_two()
                .ok_or(Error::Invalid("发光缓冲容量溢出"))?;
        }
        let grow = self.delta.capacity != self.current.capacity;
        let tables = if grow {
            Some(self.full_tables()?)
        } else {
            None
        };
        if !grow && self.current.capacity != 0 {
            let mut inverse = Inverse::new(&self.current.inverse);
            for &(key, id) in &self.removals {
                inverse.remove(key, id)?;
            }
            for &(key, id) in &self.insertions {
                inverse.insert(key, id)?;
            }
            inverse.set(
                0,
                [
                    self.delta.count as u32,
                    (self.current.inverse.len() - 2) as u32,
                    1,
                    0,
                ],
            )?;
            self.delta.inverse = inverse.edits;
        }
        let mut patches = TablePatches::default();
        if !grow {
            let records = self
                .delta
                .records
                .len()
                .checked_add(self.delta.additions.len())
                .ok_or(Error::Invalid("发光patch数量溢出"))?;
            patches.meta = reserved(records)?;
            patches.vertices = reserved(
                records
                    .checked_mul(3)
                    .ok_or(Error::Invalid("发光顶点patch数量溢出"))?,
            )?;
            patches.regions = reserved(records)?;
            for (&id, edit) in &self.delta.records {
                if edit.mask & META != 0 {
                    patches.meta.push((id as usize, edit.record.meta));
                }
                if edit.mask & VERTICES != 0 {
                    patches.vertices.extend(
                        edit.record
                            .vertices
                            .into_iter()
                            .enumerate()
                            .map(|(i, row)| (id as usize * 3 + i, row)),
                    );
                }
                if edit.mask & REGION != 0 {
                    patches.regions.push((id as usize, edit.record.region));
                }
            }
            for &(id, record) in &self.delta.additions {
                patches.meta.push((id as usize, record.meta));
                patches.vertices.extend(
                    record
                        .vertices
                        .into_iter()
                        .enumerate()
                        .map(|(i, row)| (id as usize * 3 + i, row)),
                );
                patches.regions.push((id as usize, record.region));
            }
            patches.inverse = reserved(self.delta.inverse.len())?;
            patches
                .inverse
                .extend(self.delta.inverse.iter().map(|(&i, &row)| (i, row)));
        }
        Ok((self.delta, tables, patches))
    }

    fn full_tables(&self) -> Result<Tables, Error> {
        let capacity = self.delta.capacity;
        let mut meta = reserved(capacity)?;
        let mut vertices = reserved(capacity * 3)?;
        let mut regions = reserved(capacity)?;
        meta.extend_from_slice(&self.current.meta);
        vertices.extend_from_slice(&self.current.vertices);
        regions.extend_from_slice(&self.current.regions);
        meta.resize(capacity, [0; 4]);
        vertices.resize(capacity * 3, [0.; 4]);
        regions.resize(capacity, [0; 4]);
        for (&id, edit) in &self.delta.records {
            let index = id as usize;
            if edit.mask & META != 0 {
                meta[index] = edit.record.meta;
            }
            if edit.mask & VERTICES != 0 {
                vertices[index * 3..index * 3 + 3].copy_from_slice(&edit.record.vertices);
            }
            if edit.mask & REGION != 0 {
                regions[index] = edit.record.region;
            }
        }
        for &(id, record) in &self.delta.additions {
            let index = id as usize;
            meta[index] = record.meta;
            vertices[index * 3..index * 3 + 3].copy_from_slice(&record.vertices);
            regions[index] = record.region;
        }
        let buckets = capacity
            .checked_mul(2)
            .ok_or(Error::Invalid("发光反查容量溢出"))?;
        let mut inverse = reserved(buckets + 1)?;
        inverse.resize(buckets + 1, [0; 4]);
        inverse[0] = [self.delta.count as u32, (buckets - 1) as u32, 1, 0];
        for rank in 0..self.delta.count {
            let id = self.active(rank);
            let key = meta[id as usize][3]
                .checked_add(1)
                .ok_or(Error::Invalid("发光反查身份溢出"))?;
            let mut bucket = scene_cache::hash(key) as usize & (buckets - 1);
            loop {
                let row = &mut inverse[1 + bucket];
                if row[0] == key {
                    return Err(Error::Invalid("发光quad属性槽重复"));
                }
                if row[0] == 0 {
                    *row = [key, id + 1, 0, 0];
                    break;
                }
                bucket = (bucket + 1) & (buckets - 1);
            }
        }
        Ok(Tables {
            meta,
            vertices,
            regions,
            inverse,
        })
    }
}

struct Inverse<'a> {
    current: &'a [[u32; 4]],
    edits: HashMap<usize, [u32; 4]>,
    mask: usize,
}

impl<'a> Inverse<'a> {
    fn new(current: &'a [[u32; 4]]) -> Self {
        Self {
            current,
            edits: HashMap::new(),
            mask: current.len() - 2,
        }
    }
    fn get(&self, index: usize) -> [u32; 4] {
        self.edits
            .get(&index)
            .copied()
            .unwrap_or(self.current[index])
    }
    fn set(&mut self, index: usize, row: [u32; 4]) -> Result<(), Error> {
        if row == self.current[index] {
            self.edits.remove(&index);
            Ok(())
        } else {
            mapped(&mut self.edits, index, row)
        }
    }
    fn insert(&mut self, key: u32, id: u32) -> Result<(), Error> {
        let mut bucket = scene_cache::hash(key) as usize & self.mask;
        for _ in 0..=self.mask {
            let row = self.get(bucket + 1);
            if row[0] == key {
                return Err(Error::Invalid("发光quad属性槽重复"));
            }
            if row[0] == 0 {
                return self.set(bucket + 1, [key, id + 1, 0, 0]);
            }
            bucket = (bucket + 1) & self.mask;
        }
        Err(Error::Invalid("发光反查表没有空桶"))
    }
    fn remove(&mut self, key: u32, id: u32) -> Result<(), Error> {
        let mut bucket = scene_cache::hash(key) as usize & self.mask;
        let mut found = false;
        for _ in 0..=self.mask {
            let row = self.get(bucket + 1);
            if row[0] == key {
                if row[1] != id + 1 {
                    return Err(Error::Invalid("发光反查身份不匹配"));
                }
                found = true;
                break;
            }
            if row[0] == 0 {
                break;
            }
            bucket = (bucket + 1) & self.mask;
        }
        if !found {
            return Err(Error::Invalid("发光反查身份不存在"));
        }
        let mut hole = bucket;
        bucket = (bucket + 1) & self.mask;
        for _ in 0..=self.mask {
            let row = self.get(bucket + 1);
            if row[0] == 0 {
                return self.set(hole + 1, [0; 4]);
            }
            let home = scene_cache::hash(row[0]) as usize & self.mask;
            if (bucket.wrapping_sub(home) & self.mask) >= (bucket.wrapping_sub(hole) & self.mask) {
                self.set(hole + 1, row)?;
                hole = bucket;
            }
            bucket = (bucket + 1) & self.mask;
        }
        Err(Error::Invalid("发光反查探测链没有空桶"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn record(slot: u32) -> Record {
        Record {
            meta: [u32::MAX, 0, 0, slot],
            vertices: [
                [slot as f32, 0., 1., 0.],
                [1., 0., 1., 1.],
                [0., 0., 1., 0.],
            ],
            region: [0, 0, 0, (slot << 2) | 3],
            power: 1.,
        }
    }

    fn initial(count: u32) -> (Registry, Tables) {
        let mut registry = Registry::default();
        let mut writer = Writer::new(&registry);
        for slot in 0..count {
            assert_eq!(writer.add(record(slot)).unwrap(), slot);
        }
        let (delta, tables, _) = writer.finish_for_gpu().unwrap();
        registry.reserve(&delta, tables.as_ref()).unwrap();
        let shadow = copy_tables(tables.as_ref().unwrap());
        registry.commit(delta, tables);
        (registry, shadow)
    }

    #[test]
    fn replacing_a_removed_slot_is_rejected_before_publication() {
        let (registry, _) = initial(2);
        let mut writer = Writer::new(&registry);
        writer.remove(0).unwrap();
        assert!(matches!(
            writer.replace(0, record(2)),
            Err(Error::Invalid(_))
        ));
        let (_, _, patches) = writer.finish_for_gpu().unwrap();
        assert!(patches.meta.is_empty());
    }

    fn copy_tables(tables: &Tables) -> Tables {
        // 独立CPU数组模拟设备副本，生产路径直接移交唯一wire存储。
        Tables {
            meta: tables.meta.clone(),
            vertices: tables.vertices.clone(),
            regions: tables.regions.clone(),
            inverse: tables.inverse.clone(),
        }
    }

    fn apply(tables: &mut Tables, patches: &TablePatches) {
        for &(row, value) in &patches.meta {
            tables.meta[row] = value;
        }
        for &(row, value) in &patches.vertices {
            tables.vertices[row] = value;
        }
        for &(row, value) in &patches.inverse {
            tables.inverse[row] = value;
        }
        for &(row, value) in &patches.regions {
            tables.regions[row] = value;
        }
    }

    fn lookup(table: &[[u32; 4]], key: u32) -> Option<u32> {
        let mask = table[0][1] as usize;
        let mut bucket = scene_cache::hash(key) as usize & mask;
        for _ in 0..=mask {
            let row = table[bucket + 1];
            if row[0] == key {
                return Some(row[1] - 1);
            }
            if row[0] == 0 {
                return None;
            }
            bucket = (bucket + 1) & mask;
        }
        None
    }

    fn validate(registry: &Registry, tables: &Tables) {
        assert_eq!(tables.inverse[0][0] as usize, registry.active.len());
        assert_eq!(tables.meta[..registry.meta.len()], registry.meta);
        assert_eq!(
            tables.vertices[..registry.vertices.len()],
            registry.vertices
        );
        assert_eq!(tables.regions[..registry.regions.len()], registry.regions);
        assert_eq!(tables.inverse, registry.inverse);
        let mut identities = HashSet::new();
        for (rank, &id) in registry.active.iter().enumerate() {
            assert!(identities.insert(id));
            assert_eq!(registry.ranks[id as usize], rank as u32);
            assert_eq!(registry.regions[id as usize][3] & 3, 3);
            assert_eq!(
                lookup(&tables.inverse, registry.meta[id as usize][3] + 1),
                Some(id)
            );
        }
    }

    #[test]
    fn removal_does_not_patch_another_regions_moved_slot() {
        let (mut registry, mut tables) = initial(1_000);
        let old_last = registry.active[999];
        let mut writer = Writer::new(&registry);
        writer.remove(17).unwrap();
        let (delta, grown, patches) = writer.finish_for_gpu().unwrap();
        assert!(grown.is_none());
        assert!(patches.meta.is_empty());
        assert!(patches.vertices.is_empty());
        assert!(patches.regions.is_empty());
        apply(&mut tables, &patches);
        registry.reserve(&delta, None).unwrap();
        registry.commit(delta, None);
        assert_eq!(registry.active[17], old_last);
        assert_eq!(registry.record(old_last).meta[3], 999);
        assert_eq!(registry.record(old_last).region[3] >> 2, 999);
        assert_eq!(lookup(&registry.inverse, 18), None);
        validate(&registry, &tables);
    }

    #[test]
    fn discarded_geometry_preparation_keeps_the_published_proposal() {
        let (registry, tables) = initial(4);
        let mut writer = Writer::new(&registry);
        writer.remove(0).unwrap();
        let (delta, _, patches) = writer.finish_for_gpu().unwrap();
        assert!(!patches.inverse.is_empty());
        drop(delta);
        validate(&registry, &tables);
    }

    #[test]
    fn removed_slots_are_reused_without_growing_or_moving_other_light_records() {
        let (mut registry, mut tables) = initial(16);
        let mut writer = Writer::new(&registry);
        writer.remove(4).unwrap();
        assert_eq!(writer.add(record(1_000)).unwrap(), 4);
        let (delta, grown, patches) = writer.finish_for_gpu().unwrap();
        assert!(grown.is_none());
        assert_eq!(patches.meta.len(), 1);
        assert_eq!(patches.vertices.len(), 3);
        apply(&mut tables, &patches);
        registry.reserve(&delta, None).unwrap();
        registry.commit(delta, None);
        assert_eq!(registry.capacity, 16);
        assert_eq!(registry.active[15], 4);
        assert_eq!(lookup(&registry.inverse, 5), None);
        assert_eq!(lookup(&registry.inverse, 1_001), Some(4));
        validate(&registry, &tables);
    }

    #[test]
    fn appending_into_reserved_tail_only_writes_the_new_slot() {
        let (mut registry, mut tables) = initial(3);
        let mut writer = Writer::new(&registry);
        assert_eq!(writer.add(record(3)).unwrap(), 3);
        let (delta, grown, patches) = writer.finish_for_gpu().unwrap();
        assert!(grown.is_none());
        assert_eq!(patches.meta.len(), 1);
        assert_eq!(patches.vertices.len(), 3);
        assert_eq!(patches.regions.len(), 1);
        apply(&mut tables, &patches);
        registry.reserve(&delta, None).unwrap();
        registry.commit(delta, None);
        assert_eq!(registry.span, 4);
        assert_eq!(registry.capacity, 16);
        validate(&registry, &tables);
    }

    #[test]
    fn transferred_tables_keep_storage_identity_and_reserve_active_capacity_for_sparse_appends() {
        let mut registry = Registry::default();
        let mut writer = Writer::new(&registry);
        for slot in 0..3 {
            writer.add(record(slot)).unwrap();
        }
        let (delta, tables, _) = writer.finish_for_gpu().unwrap();
        let tables = tables.unwrap();
        let storage = (
            tables.meta.as_ptr(),
            tables.vertices.as_ptr(),
            tables.regions.as_ptr(),
            tables.inverse.as_ptr(),
        );
        registry.reserve(&delta, Some(&tables)).unwrap();
        registry.commit(delta, Some(tables));
        assert_eq!(
            (
                registry.meta.as_ptr(),
                registry.vertices.as_ptr(),
                registry.regions.as_ptr(),
                registry.inverse.as_ptr()
            ),
            storage
        );
        let active = registry.active.as_ptr();
        for slot in 3..16 {
            let mut writer = Writer::new(&registry);
            writer.add(record(slot)).unwrap();
            let (delta, tables, _) = writer.finish_for_gpu().unwrap();
            assert!(tables.is_none());
            registry.reserve(&delta, None).unwrap();
            registry.commit(delta, None);
            assert_eq!(registry.active.as_ptr(), active);
            assert_eq!(
                (
                    registry.meta.as_ptr(),
                    registry.vertices.as_ptr(),
                    registry.regions.as_ptr(),
                    registry.inverse.as_ptr()
                ),
                storage
            );
        }
    }

    #[test]
    fn backshift_deletion_preserves_colliding_wraparound_probe_chains() {
        let keys: Vec<_> = (1..100_000)
            .filter(|&key| scene_cache::hash(key) & 31 == 31)
            .take(16)
            .collect();
        let mut registry = Registry::default();
        let mut writer = Writer::new(&registry);
        for &key in &keys {
            writer.add(record(key - 1)).unwrap();
        }
        let (delta, tables, _) = writer.finish_for_gpu().unwrap();
        registry.reserve(&delta, tables.as_ref()).unwrap();
        let mut shadow = copy_tables(tables.as_ref().unwrap());
        registry.commit(delta, tables);
        let mut writer = Writer::new(&registry);
        for id in [1, 3, 5, 7, 9, 11, 13, 15] {
            writer.remove(id).unwrap();
        }
        let (delta, grown, patches) = writer.finish_for_gpu().unwrap();
        assert!(grown.is_none());
        apply(&mut shadow, &patches);
        registry.reserve(&delta, None).unwrap();
        registry.commit(delta, None);
        for (id, &key) in keys.iter().enumerate() {
            assert_eq!(
                lookup(&registry.inverse, key),
                if id % 2 == 0 { Some(id as u32) } else { None }
            );
        }
        validate(&registry, &shadow);
    }

    #[test]
    fn capacity_growth_rebuilds_the_table_and_keeps_existing_data_identities() {
        let (mut registry, _) = initial(16);
        let mut writer = Writer::new(&registry);
        assert_eq!(writer.add(record(16)).unwrap(), 16);
        let (delta, tables, _) = writer.finish_for_gpu().unwrap();
        assert_eq!(tables.as_ref().unwrap().meta.len(), 32);
        registry.reserve(&delta, tables.as_ref()).unwrap();
        let shadow = copy_tables(tables.as_ref().unwrap());
        registry.commit(delta, tables);
        assert_eq!(registry.active, (0..17).collect::<Vec<_>>());
        validate(&registry, &shadow);
    }

    #[test]
    fn duplicate_quad_slots_fail_before_changing_the_published_registry() {
        let (registry, tables) = initial(8);
        let mut writer = Writer::new(&registry);
        writer.replace(1, record(0)).unwrap();
        assert!(writer.finish_for_gpu().is_err());
        validate(&registry, &tables);
    }

    #[test]
    fn long_churn_reuses_slots_and_keeps_reverse_lookup_without_tombstones() {
        let (mut registry, mut tables) = initial(32);
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        for sequence in 0..1_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let rank = state as usize % registry.active.len();
            let id = registry.active[rank];
            let mut writer = Writer::new(&registry);
            writer.remove(id).unwrap();
            assert_eq!(writer.add(record(10_000 + sequence)).unwrap(), id);
            let (delta, grown, patches) = writer.finish_for_gpu().unwrap();
            assert!(grown.is_none());
            apply(&mut tables, &patches);
            registry.reserve(&delta, None).unwrap();
            registry.commit(delta, None);
            assert_eq!(registry.capacity, 32);
            validate(&registry, &tables);
        }
    }

    #[test]
    fn a_recycled_slot_publishes_its_new_region_local_index() {
        let (mut registry, mut tables) = initial(7);
        let mut writer = Writer::new(&registry);
        writer.remove(2).unwrap();
        let id = writer.add(record(100)).unwrap();
        let (delta, grown, patches) = writer.finish_for_gpu().unwrap();
        assert!(grown.is_none());
        apply(&mut tables, &patches);
        registry.reserve(&delta, None).unwrap();
        registry.commit(delta, None);
        assert_eq!(lookup(&tables.inverse, 101), Some(id));
        assert_eq!(tables.regions[id as usize][3] >> 2, 100);
        validate(&registry, &tables);
    }
}
