//! 稳定几何域的身份目录；源键、源版本和GPU位置由应用适配，不解释材质或几何来源。
use crate::Error;
use std::sync::atomic::Ordering;

/// 一份真实源域；版本须由源所有者精确维护，不能用可能碰撞的摘要替代。
#[derive(Clone, Copy, Debug)]
pub struct GeometrySource<K> {
    /// 场景域内唯一、有序的源键，例如应用的region与slab。
    pub key: K,
    /// 非零版本；相同键与版本承诺primitive顺序和几何保持一致。
    pub revision: u64,
    /// 当前恢复位置的三个不透明word；可随槽压缩改变，不改变身份。
    pub location: [u32; 3],
}

/// 已建立身份的源域；token只在成功提交后成为公开身份。
#[derive(Clone, Copy, Debug)]
pub struct GeometryRecord<K> {
    /// 应用提供的来源与当前位置。
    pub source: GeometrySource<K>,
    /// 非零且在本所有者内不回绕、不复用的GPU身份。
    pub token: u32,
}

/// 独占成功发布的几何域；只保存域元数据，不复制顶点或材质。
pub struct GeometryDirectory<K> {
    records: Vec<GeometryRecord<K>>,
    words: Vec<[u32; 4]>,
    next: u32,
    owner: u64,
    generation: u64,
}

/// 尚未发布的完整候选；失败或取消不推进发行计数，也不改变当前GPU目录。
pub struct GeometryUpdate<K> {
    records: Vec<GeometryRecord<K>>,
    words: Vec<[u32; 4]>,
    next: u32,
    owner: u64,
    generation: u64,
}

impl<K> GeometryUpdate<K> {
    /// 候选的有序记录，供调用方把token写入真实命中恢复数据。
    pub const fn records(&self) -> &[GeometryRecord<K>] {
        self.records.as_slice()
    }
    /// 与 [`crate::directory`] 相同的完整编码；查询比较完整token，hash碰撞不造成误认。
    pub const fn words(&self) -> &[[u32; 4]] {
        self.words.as_slice()
    }
}

/// 已校验的唯一发布阶段；GPU原子写入成功后提交，丢弃则保留当前目录。
pub struct GeometryPublication<'a, K> {
    directory: &'a mut GeometryDirectory<K>,
    update: GeometryUpdate<K>,
    generation: u64,
}
impl<K> GeometryPublication<'_, K> {
    /// 候选GPU编码；持有此阶段时不能并发发布另一个候选。
    pub const fn words(&self) -> &[[u32; 4]] {
        self.update.words.as_slice()
    }
    /// 移交已准备存储；不分配、不失败，不代替GPU完成证明或资源退役。
    pub fn commit(self) {
        let Self {
            directory,
            update,
            generation,
        } = self;
        directory.records = update.records;
        directory.words = update.words;
        directory.next = update.next;
        directory.generation = generation;
    }
}
impl<K: Copy + Ord> GeometryDirectory<K> {
    /// 创建新身份域；0保留为无效，耗尽不回绕。
    /// # Errors
    /// 所有者身份耗尽或初始GPU目录分配失败。
    pub fn new() -> Result<Self, Error> {
        let owner = crate::NEXT_OWNER
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| Error::Capacity)?;
        Ok(Self {
            records: Vec::new(),
            words: crate::directory(&[])?,
            next: 1,
            owner,
            generation: 0,
        })
    }
    /// 当前已发布的有序域。
    pub const fn records(&self) -> &[GeometryRecord<K>] {
        self.records.as_slice()
    }
    /// 当前GPU目录编码；只有活动域占空间，不随累计发行数量增长。
    pub const fn words(&self) -> &[[u32; 4]] {
        self.words.as_slice()
    }
    /// 合并有序源域，精确复用相同key/revision的token，并更新其当前位置。
    ///
    /// 删除后再次出现的键发行新token；源版本变化同样发行新token。
    /// 源版本正确性属于应用所有者可信边界；不能用指针地址或概率hash冒充版本。
    /// # Errors
    /// 源键乱序/重复、版本为0、身份/地址耗尽或分配失败；当前目录不变。
    pub fn begin(&self, sources: &[GeometrySource<K>]) -> Result<GeometryUpdate<K>, Error> {
        if sources.iter().any(|source| source.revision == 0)
            || sources.windows(2).any(|pair| pair[0].key >= pair[1].key)
        {
            return Err(Error::Identity);
        }
        let mut records = Vec::new();
        records.try_reserve_exact(sources.len())?;
        let mut encoded = Vec::new();
        encoded.try_reserve_exact(sources.len())?;
        let mut old = 0;
        let mut next = self.next;
        for &source in sources {
            while self
                .records
                .get(old)
                .is_some_and(|entry| entry.source.key < source.key)
            {
                old += 1;
            }
            let token = if let Some(entry) = self.records.get(old)
                && entry.source.key == source.key
                && entry.source.revision == source.revision
            {
                entry.token
            } else {
                let token = next;
                next = next.checked_add(1).ok_or(Error::Capacity)?;
                token
            };
            records.push(GeometryRecord { source, token });
            encoded.push([
                source.location[0],
                source.location[1],
                source.location[2],
                token,
            ]);
        }
        Ok(GeometryUpdate {
            records,
            words: crate::directory(&encoded)?,
            next,
            owner: self.owner,
            generation: self.generation,
        })
    }
    /// 在任何GPU写入前校验来源与代次，建立提交不失败的阶段。
    /// # Errors
    /// 候选来自别的所有者、已被提交取代或目录代次耗尽。
    pub fn publication(
        &mut self,
        update: GeometryUpdate<K>,
    ) -> Result<GeometryPublication<'_, K>, Error> {
        if update.owner != self.owner || update.generation != self.generation {
            return Err(Error::Stale);
        }
        let generation = self.generation.checked_add(1).ok_or(Error::Capacity)?;
        Ok(GeometryPublication {
            directory: self,
            update,
            generation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(key: u32, revision: u64, at: u32) -> GeometrySource<u32> {
        GeometrySource {
            key,
            revision,
            location: [at, 1, 0],
        }
    }
    #[test]
    fn moving_a_domain_preserves_its_identity() {
        let mut owner = GeometryDirectory::new().unwrap();
        let update = owner.begin(&[source(1, 3, 5)]).unwrap();
        let token = update.records()[0].token;
        owner.publication(update).unwrap().commit();
        let update = owner.begin(&[source(1, 3, 20)]).unwrap();
        assert_eq!(update.records()[0].token, token);
        assert_eq!(update.records()[0].source.location[0], 20);
    }
    #[test]
    fn a_changed_slab_does_not_invalidate_its_neighbor() {
        let mut owner = GeometryDirectory::new().unwrap();
        let update = owner.begin(&[source(1, 3, 0), source(2, 4, 10)]).unwrap();
        let old = update.records().to_vec();
        owner.publication(update).unwrap().commit();
        let update = owner.begin(&[source(1, 5, 0), source(2, 4, 11)]).unwrap();
        assert_ne!(update.records()[0].token, old[0].token);
        assert_eq!(update.records()[1].token, old[1].token);
    }
    #[test]
    fn canceled_updates_do_not_issue_public_identities() {
        let mut owner = GeometryDirectory::new().unwrap();
        let canceled = owner.begin(&[source(1, 1, 0)]).unwrap();
        let token = canceled.records()[0].token;
        drop(canceled);
        assert!(owner.records().is_empty());
        let committed = owner.begin(&[source(1, 1, 0)]).unwrap();
        assert_eq!(committed.records()[0].token, token);
        owner.publication(committed).unwrap().commit();
        let empty = owner.begin(&[]).unwrap();
        owner.publication(empty).unwrap().commit();
        assert_ne!(
            owner.begin(&[source(1, 1, 0)]).unwrap().records()[0].token,
            token
        );
    }
}
