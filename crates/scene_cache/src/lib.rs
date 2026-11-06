//! 场景资源记录的增量事务；不决定几何来源、材质、分块尺度或GPU提交。
//!
//! [`Cache`]独占当前记录、属性槽范围和单调身份。更新只复制范围元数据并收集
//! [`Prepared`]，成功后移动复用记录并发布；失败丢弃更新保留当前资源目录。
//! GPU所有者负责命令保活、上传顺序及完成证明；槽回收不许可并发host覆盖旧GPU读取。
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

/// 容量、资源事务或活动身份输入无效。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 索引、槽范围或所有者／世代／身份计数耗尽。
    #[error("场景资源索引、槽范围或身份计数耗尽")]
    Capacity,
    /// CPU元数据分配失败。
    #[error(transparent)]
    Allocation(#[from] std::collections::TryReserveError),
    /// 复用索引须属于当前目录并严格递增。
    #[error("场景事务包含越界或重复/乱序复用索引")]
    Reuse,
    /// 活动身份必须非零且唯一。
    #[error("场景活动身份为零或重复")]
    Identity,
    /// 更新来自其他所有者或已经被另一次成功发布取代。
    #[error("场景更新所有者或目录世代不匹配")]
    Stale,
}

mod slots;
pub use slots::Slots;
mod geometry;
pub use geometry::{
    GeometryDirectory, GeometryPublication, GeometryRecord, GeometrySource, GeometryUpdate,
};

/// 资源替换身份序列；0保留为无效值，不因目录压缩或槽复用而复活。
pub struct Identities {
    next: u32,
    owner: u64,
    generation: u64,
}
impl Identities {
    /// 预留非零身份；溢出返回错误，不回绕。
    pub fn take(&mut self) -> Result<u32, Error> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or(Error::Capacity)?;
        Ok(id)
    }
}

/// 待发布目录项；复用索引指向同一Cache当前记录，Built独占新资源记录。
pub enum Prepared<T> {
    /// 原目录索引；最终目录须保持这些索引严格递增。
    Reused(usize),
    /// 已构造的新记录；失败时随事务丢弃。
    Built(T),
}
impl<T> Prepared<T> {
    /// 在发布前访问新旧记录；索引由同一Cache的目录查询建立。
    pub fn get<'a>(&'a self, current: &'a [T]) -> &'a T {
        match self {
            Self::Reused(index) => &current[*index],
            Self::Built(value) => value,
        }
    }
}

/// 尚未发布的范围、身份与资源记录；不借用原目录，允许调度层分批构造BLAS。
pub struct Update<T> {
    slots: Slots,
    identities: Identities,
    entries: Vec<Prepared<T>>,
    next: Vec<T>,
    owner: u64,
    generation: u64,
}
impl<T> Update<T> {
    /// 独占各准备输出；调用者负责在提交前完成所有Built记录。
    pub fn parts_mut(&mut self) -> (&mut Slots, &mut Vec<Prepared<T>>, &mut Identities) {
        (&mut self.slots, &mut self.entries, &mut self.identities)
    }
}

/// 独占一个应用场景域的资源目录；T通常拥有BLAS句柄与应用元数据。
pub struct Cache<T> {
    entries: Vec<T>,
    slots: Slots,
    identities: Identities,
    owner: u64,
    generation: u64,
}

/// 已校验且独占目录的发布阶段；丢弃时保留当前目录，提交不再分配或返回错误。
///
/// 设备所有者可在持有此阶段时执行一次原子属性写入；只有写入成功才调用
/// [`Self::commit`]。借用阻止其间改变目录世代，不代表GPU读取已完成。
/// 提交可能析构被替换的`T`；跨设备发布的调用者须保证其析构不panic。
pub struct Publication<'a, T> {
    cache: &'a mut Cache<T>,
    update: Update<T>,
    generation: u64,
}

impl<T> Publication<'_, T> {
    /// 移动已准备记录并发布；全部容量、身份和范围回收已在准备阶段完成。
    pub fn commit(self) {
        let Self {
            cache,
            mut update,
            generation,
        } = self;
        let mut current = std::mem::take(&mut cache.entries)
            .into_iter()
            .enumerate()
            .peekable();
        for item in update.entries {
            update.next.push(match item {
                Prepared::Built(value) => value,
                Prepared::Reused(index) => {
                    while current.peek().is_some_and(|(old, _)| *old < index) {
                        current.next();
                    }
                    current.next().expect("准备阶段已校验复用索引").1
                }
            });
        }
        cache.entries = update.next;
        update.slots.generation = generation;
        cache.slots = update.slots;
        update.identities.generation = generation;
        cache.identities = update.identities;
        cache.generation = generation;
    }
}
impl<T> Cache<T> {
    /// 建立新的资源身份域；仅分配进程唯一所有者编号，不创建GPU资源。
    /// # Errors
    /// 进程所有者编号耗尽时返回容量错误。
    pub fn new() -> Result<Self, Error> {
        let owner = NEXT_OWNER
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Error::Capacity)?;
        Ok(Self {
            entries: Vec::new(),
            slots: Slots::new(owner),
            identities: Identities {
                next: 1,
                owner,
                generation: 0,
            },
            owner,
            generation: 0,
        })
    }
}
impl<T> Cache<T> {
    /// 当前成功发布的记录；顺序由调用者保持，未包含准备中的资源。
    pub const fn entries(&self) -> &[T] {
        self.entries.as_slice()
    }

    /// 当前目录与空闲范围的显式保留容量，不含T拥有的间接数据或GPU内存。
    pub const fn retained_bytes(&self) -> usize {
        self.entries.capacity() * size_of::<T>() + self.slots.retained_bytes()
    }

    /// 创建独立更新，预留最终目录项；失败不修改当前槽、身份或资源。
    pub fn begin(&self, capacity: usize) -> Result<Update<T>, Error> {
        let mut entries = Vec::new();
        entries.try_reserve_exact(capacity)?;
        let mut next = Vec::new();
        next.try_reserve_exact(capacity)?;
        Ok(Update {
            slots: self.slots.copy()?,
            identities: Identities {
                next: self.identities.next,
                owner: self.owner,
                generation: self.generation,
            },
            entries,
            next,
            owner: self.owner,
            generation: self.generation,
        })
    }

    /// 发布全部记录并回收未复用项的槽；range须返回记录此前由本Cache预留的范围。
    /// GPU提交须已成功排定，旧命令的资源保活仍由设备所有者维护。
    /// # Errors
    /// 分配或复用索引校验失败保留原目录；调用者保证Built完整且range互不重叠。
    pub fn commit(
        &mut self,
        update: Update<T>,
        range: impl Fn(&T) -> Range<u32>,
    ) -> Result<(), Error> {
        self.prepare(update, range)?.commit();
        Ok(())
    }

    /// 校验完整更新并准备槽回收；失败或丢弃返回值均保留当前目录。
    ///
    /// `range`须返回此前由本目录预留的范围，所有Built记录已完整且范围互不重叠。
    /// 返回阶段独占此目录；用于将可失败的设备准备与不可失败的目录发布连接起来。
    pub fn prepare(
        &mut self,
        mut update: Update<T>,
        range: impl Fn(&T) -> Range<u32>,
    ) -> Result<Publication<'_, T>, Error> {
        if update.owner != self.owner
            || update.generation != self.generation
            || update.slots.owner != self.owner
            || update.slots.generation != self.generation
            || update.identities.owner != self.owner
            || update.identities.generation != self.generation
        {
            return Err(Error::Stale);
        }
        let generation = self.generation.checked_add(1).ok_or(Error::Capacity)?;
        if update.entries.len() > update.next.capacity() {
            return Err(Error::Reuse);
        }
        let mut previous = None;
        for item in &update.entries {
            if let Prepared::Reused(index) = item {
                if *index >= self.entries.len() || previous.is_some_and(|old| old >= *index) {
                    return Err(Error::Reuse);
                }
                previous = Some(*index);
            }
        }
        let mut reused = update
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Prepared::Reused(index) => Some(*index),
                Prepared::Built(_) => None,
            })
            .peekable();
        for (index, entry) in self.entries.iter().enumerate() {
            if reused.peek() == Some(&index) {
                reused.next();
            } else {
                update.slots.release(range(entry))?;
            }
        }
        update.slots.merge()?;
        Ok(Publication {
            cache: self,
            update,
            generation,
        })
    }
}

/// 有序目录中的精确revision匹配；key和revision来自当前资源与调用者真实输入。
pub fn find_revision<T, K: Ord>(
    entries: &[T],
    key: K,
    revision: u64,
    key_of: impl Fn(&T) -> K,
    revision_of: impl Fn(&T) -> u64,
) -> Option<usize> {
    entries
        .binary_search_by_key(&key, key_of)
        .ok()
        .filter(|&index| revision_of(&entries[index]) == revision)
}

/// 与Slang活动身份目录一致的32位hash。
pub const fn hash(mut key: u32) -> u32 {
    key ^= key >> 16;
    key = key.wrapping_mul(0x7feb_352d);
    key ^= key >> 15;
    key = key.wrapping_mul(0x846c_a68b);
    key ^ (key >> 16)
}

/// 编码header、实例元数据和开放寻址表；entry.w是非零唯一身份。
/// 输出仅覆盖当前活动域，删除项不保留映射，目录重排不改变稳定身份。
pub fn directory(entries: &[[u32; 4]]) -> Result<Vec<[u32; 4]>, Error> {
    let count = u32::try_from(entries.len()).map_err(|_| Error::Capacity)?;
    let slots = count
        .checked_mul(2)
        .and_then(u32::checked_next_power_of_two)
        .ok_or(Error::Capacity)?
        .max(2);
    let base = count.checked_add(1).ok_or(Error::Capacity)?;
    let length = base.checked_add(slots / 2).ok_or(Error::Capacity)? as usize;
    let mut words = Vec::new();
    words.try_reserve_exact(length)?;
    words.push([count, slots - 1, base, 0]);
    words.extend_from_slice(entries);
    words.resize(length, [0; 4]);
    for (index, entry) in entries.iter().enumerate() {
        let key = entry[3];
        if key == 0 {
            return Err(Error::Identity);
        }
        let mut slot = hash(key) & (slots - 1);
        loop {
            let row = &mut words[(base + slot / 2) as usize];
            let lane = (slot & 1) as usize * 2;
            if row[lane] == 0 {
                row[lane] = key;
                row[lane + 1] = index as u32 + 1;
                break;
            }
            if row[lane] == key {
                return Err(Error::Identity);
            }
            slot = (slot + 1) & (slots - 1);
        }
    }
    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Resource {
        range: Range<u32>,
        id: u32,
    }
    #[test]
    fn rejected_update_preserves_identity_and_slots() -> Result<(), Error> {
        let mut cache = Cache::<Resource>::new()?;
        let mut failed = cache.begin(1)?;
        let (slots, _, ids) = failed.parts_mut();
        assert_eq!(slots.take(4)?, 0..4);
        assert_eq!(ids.take()?, 1);
        drop(failed);
        let mut next = cache.begin(1)?;
        let (slots, entries, ids) = next.parts_mut();
        entries.push(Prepared::Built(Resource {
            range: slots.take(2)?,
            id: ids.take()?,
        }));
        cache.commit(next, |entry| entry.range.clone())?;
        assert_eq!(cache.entries()[0].id, 1);
        assert_eq!(cache.entries()[0].range, 0..2);
        Ok(())
    }
    #[test]
    fn replacement_cannot_overwrite_old_slots_before_commit() -> Result<(), Error> {
        let mut cache = Cache::<Resource>::new()?;
        for expected in [0..4, 4..8, 0..4] {
            let mut update = cache.begin(1)?;
            let (slots, entries, ids) = update.parts_mut();
            let range = slots.take(4)?;
            assert_eq!(range, expected);
            entries.push(Prepared::Built(Resource {
                range,
                id: ids.take()?,
            }));
            cache.commit(update, |entry| entry.range.clone())?;
        }
        assert_eq!(cache.entries()[0].id, 3);
        Ok(())
    }

    #[test]
    fn abandoned_publication_preserves_resources_and_reusable_ranges() -> Result<(), Error> {
        let mut cache = Cache::<Resource>::new()?;
        let mut initial = cache.begin(1)?;
        let (slots, entries, _) = initial.parts_mut();
        entries.push(Prepared::Built(Resource {
            range: slots.take(4)?,
            id: 1,
        }));
        cache.commit(initial, |entry| entry.range.clone())?;
        let mut candidate = cache.begin(1)?;
        let (slots, entries, _) = candidate.parts_mut();
        entries.push(Prepared::Built(Resource {
            range: slots.take(4)?,
            id: 2,
        }));
        drop(cache.prepare(candidate, |entry| entry.range.clone())?);
        assert_eq!(cache.entries()[0].id, 1);
        let mut retry = cache.begin(1)?;
        let (slots, entries, _) = retry.parts_mut();
        let range = slots.take(4)?;
        assert_eq!(range, 4..8);
        entries.push(Prepared::Built(Resource { range, id: 3 }));
        cache.prepare(retry, |entry| entry.range.clone())?.commit();
        assert_eq!(cache.entries()[0].id, 3);
        Ok(())
    }

    #[test]
    fn freed_ranges_merge_and_reuse_without_live_aliases() -> Result<(), Error> {
        let mut slots = Cache::<Resource>::new()?.begin(0)?.slots;
        let a = slots.take(5)?;
        let b = slots.take(7)?;
        let c = slots.take(3)?;
        slots.release(a)?;
        slots.release(b)?;
        slots.merge()?;
        assert_eq!(slots.take(10)?, 0..10);
        assert_eq!(c, 12..15);
        slots.release(c)?;
        slots.merge()?;
        assert_eq!(slots.end(), 10);
        Ok(())
    }

    #[test]
    fn stale_update_cannot_replace_a_new_generation() -> Result<(), Error> {
        let mut cache = Cache::<Resource>::new()?;
        let mut initial = cache.begin(1)?;
        let (slots, entries, ids) = initial.parts_mut();
        entries.push(Prepared::Built(Resource {
            range: slots.take(4)?,
            id: ids.take()?,
        }));
        cache.commit(initial, |entry| entry.range.clone())?;
        let stale = cache.begin(1)?;
        let mut next = cache.begin(1)?;
        next.parts_mut().1.push(Prepared::Reused(0));
        cache.commit(next, |entry| entry.range.clone())?;
        assert!(matches!(
            cache.commit(stale, |entry| entry.range.clone()),
            Err(Error::Stale)
        ));
        assert_eq!(cache.entries()[0].id, 1);
        assert_eq!(cache.entries()[0].range, 0..4);
        Ok(())
    }

    #[test]
    fn update_cannot_cross_resource_owners() -> Result<(), Error> {
        let original = Cache::<Resource>::new()?;
        let mut other = Cache::<Resource>::new()?;
        let foreign = original.begin(0)?;
        assert!(matches!(
            other.commit(foreign, |entry| entry.range.clone()),
            Err(Error::Stale)
        ));
        let next = other.begin(0)?;
        other.commit(next, |entry| entry.range.clone())?;
        Ok(())
    }

    #[test]
    fn exchanged_slot_state_cannot_cross_owners() -> Result<(), Error> {
        let mut original = Cache::<Resource>::new()?;
        let other = Cache::<Resource>::new()?;
        let mut first = original.begin(0)?;
        let mut second = other.begin(0)?;
        std::mem::swap(first.parts_mut().0, second.parts_mut().0);
        assert!(matches!(
            original.commit(first, |entry| entry.range.clone()),
            Err(Error::Stale)
        ));
        let next = original.begin(0)?;
        original.commit(next, |entry| entry.range.clone())?;
        Ok(())
    }

    #[test]
    fn exchanged_identity_state_cannot_cross_owners() -> Result<(), Error> {
        let mut original = Cache::<Resource>::new()?;
        let other = Cache::<Resource>::new()?;
        let mut first = original.begin(0)?;
        let mut second = other.begin(0)?;
        std::mem::swap(first.parts_mut().2, second.parts_mut().2);
        assert!(matches!(
            original.commit(first, |entry| entry.range.clone()),
            Err(Error::Stale)
        ));
        let mut next = original.begin(1)?;
        let (slots, entries, ids) = next.parts_mut();
        entries.push(Prepared::Built(Resource {
            range: slots.take(1)?,
            id: ids.take()?,
        }));
        original.commit(next, |entry| entry.range.clone())?;
        assert_eq!(original.entries()[0].id, 1);
        Ok(())
    }
}
