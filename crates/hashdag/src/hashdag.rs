use crate::ZOrder20x3;

#[cfg(not(verus_only))]
use crate::identity;
use crate::node::{Child, NodeIdx};
use crate::pool::Pool;
use crate::write::frames::WriteScratch;
#[cfg(verus_only)]
use hash_dag_std::identity;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "proof/dag.rs"]
mod proof;
#[cfg(verus_only)]
#[path = "proof/dag_view.rs"]
pub(crate) mod view_proof;

pub use crate::layout::{DAG_DEPTH, NODE_FANOUT, ROOT_LOD};

pub use crate::error::Error;
pub use crate::roots::RootId;
use crate::roots::RootSlots;

/// 稀疏体素 DAG，10 个分层池各存储一个 64 叉层级的节点。
///
/// 同一实例同层的完整节点内容唯一，构造中的完整节点同样参与去重。
/// 所有 64 个同值叶折叠为一个叶，并向上传播；相同非均匀子节点只共享，
/// 不消除其空间层级。节点入池后不可变，空闲槽的旧字节不属于存活节点。
///
/// 写入串行且独占：构造持有保护新节点，成功发布根或失败后统一释放。
/// 可恢复构建失败保留原根；已增长的池和 scratch 容量不回滚。
#[cfg_attr(verus_only, verus_verify)]
pub struct HashDag {
    pub(super) pools: [Pool; DAG_DEPTH as usize],
    pub(super) roots: RootSlots,
    #[cfg(verus_only)]
    pub(super) owner_ticket: Tracked<identity::Ticket>,
    // 首个 Ptr 发布前由写边界准备；容量随所有者保留，释放路径不分配。
    pub(super) scratch_stack: Vec<(NodeIdx, u8)>,
    pub(super) scratch_write: WriteScratch,
}

#[cfg_attr(verus_only, verus_verify)]
impl crate::diff::DiffGraph for HashDag {
    fn root_child(&self, root: RootId) -> Result<Child, Error> {
        self.roots.child(root)
    }

    fn slots(&self, level: u8) -> usize {
        self.pools[usize::from(level)].arrays().0.len()
    }

    fn node(&self, level: u8, index: NodeIdx) -> Option<crate::node::NodeRef<'_>> {
        #[cfg(verus_only)]
        proof! { self.pools@[level as int].read_node_at(crate::node::proof::idx(index)); }
        self.pools[usize::from(level)].get(index)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl HashDag {
    /// 原生 64 叉层数；每层消费每轴两位。
    pub const DEPTH: u8 = DAG_DEPTH;
    /// 每轴坐标位数。
    pub const AXIS_BITS: u8 = ZOrder20x3::BIT_DEPTH;
    /// 一个原生节点的子项数。
    pub const FANOUT: usize = NODE_FANOUT;

    /// 创建持有默认零值 root 的独占节点池；写缓冲在首次写入时准备。
    ///
    /// # Errors
    ///
    /// 实例身份耗尽或容器容量申请失败时返回错误，保留底层分配来源。
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result is Ok ==> result->Ok_0.valid() && result->Ok_0.identity_owned()
            && result->Ok_0.root_result(result->Ok_0.default_id(), Ok(Child::Leaf(0))),
    ))]
    pub fn new() -> Result<Self, Error> {
        let owner = match identity::Owner::new() {
            Ok(owner) => owner,
            Err(()) => return Err(Error::IdentityExhausted),
        };
        let roots = RootSlots::new(owner.number())?;
        #[cfg(verus_only)]
        let owner_ticket = owner.into_ticket();
        let dag = Self {
            pools: crate::layers::empty(),
            roots,
            #[cfg(verus_only)]
            owner_ticket,
            scratch_stack: Vec::new(),
            scratch_write: WriteScratch::new(),
        };
        #[cfg(verus_only)]
        proof! { proof::initialized(&dag); }
        Ok(dag)
    }
}

impl HashDag {
    /// 返回由实例保活且不能单独释放的默认 root；初始值为零。
    #[must_use]
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == self.default_id(),
        no_unwind
    ))]
    pub const fn root(&self) -> RootId {
        self.roots.default_root()
    }

    /// 读取 root 的直接子节点。
    ///
    /// # Errors
    ///
    /// root 属于其他实例或句柄已失效时返回错误。
    #[inline]
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures self.root_result(root, result),
        no_unwind
    ))]
    pub fn root_child(&self, root: RootId) -> Result<Child, Error> {
        self.roots.child(root)
    }

    /// 当前保留的堆后备字节数，按池、root 和 scratch 的实际容量合计。
    ///
    /// 包含去重表控制区和对齐，释放节点后保留的空槽仍计入；
    /// 不含实例自身、allocator 元数据、进程栈和设备内存。
    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.pools.iter().map(Pool::retained_bytes).sum::<usize>()
            + self.roots.retained_bytes()
            + self.scratch_stack.capacity() * size_of::<(NodeIdx, u8)>()
            + self.scratch_write.retained_bytes()
    }

    /// 复制 root 引用，返回新的 root 句柄。
    ///
    /// # Errors
    ///
    /// root 属于其他实例、失效、节点缺失、引用溢出或容量申请失败时返回错误。
    /// 先准备 root 槽与回收容量；失败不会增加节点引用。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures proof::forked(old(self), final(self), root, result),
            old(self).identity_owned() ==> final(self).identity_owned(),
    ))]
    // 失败分支需在返回前连接原图不变的擦除证明。
    #[allow(clippy::question_mark)]
    #[inline]
    pub fn fork_root(&mut self, root: RootId) -> Result<RootId, Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let child = self.root_child(root)?;
        if let Err(error) = self.roots.prepare() {
            #[cfg(verus_only)]
            proof! { proof::logical_preserved(&before, self); }
            return Err(error);
        }
        #[cfg(verus_only)]
        proof! { proof::logical_preserved(&before, self); proof::root_fact(self, root, child); }
        if let Err(error) = retain_root(&mut self.pools, child) {
            #[cfg(verus_only)]
            proof! { proof::logical_preserved(&before, self); }
            return Err(error);
        }
        let key = self.roots.insert(child);
        #[cfg(verus_only)]
        proof! { proof::fork_committed(&before, self, root, child, key); }
        Ok(key)
    }

    /// 释放非默认 root。
    ///
    /// # Errors
    ///
    /// root 属于其他实例、默认 root、失效、版本饱和或内部节点缺失时返回错误。
    /// 版本饱和时保留 root 及其全部节点引用，调用方仍可读取或写入该 root。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures proof::released(old(self), final(self), root, result),
            old(self).identity_owned() ==> final(self).identity_owned(),
    ))]
    #[inline]
    pub fn release_root(&mut self, root: RootId) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        let (child, generation) = self.roots.prepare_release(root)?;
        if let Child::Ptr(idx) = child {
            #[cfg(verus_only)]
            proof! { proof::root_eligible(self, root, idx); }
            crate::rc::rc_dec_or_remove(&mut self.pools, idx, ROOT_LOD, &mut self.scratch_stack)?;
        }
        self.roots.retire(root, generation);
        #[cfg(verus_only)]
        proof! { proof::released_committed(&before, self, root, child); }
        Ok(())
    }

    /// write_root 已校验 root，独占构建只修改池；发布时沿用其 root 不变量。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).building(), old(self).roots.lookup(root) == Some(old_root),
            crate::state::child_valid(crate::state::pools_view(&old(self).pools), new_root, 10),
        ensures final(self).publish_result(old(self), root, old_root, new_root, result),
            old(self).identity_owned() ==> final(self).identity_owned(),
    ))]
    pub(super) fn replace_root(
        &mut self,
        root: RootId,
        old_root: Child,
        new_root: Child,
    ) -> Result<(), Error> {
        #[cfg(verus_only)]
        proof_decl! { let ghost before = *self; }
        if old_root.same(new_root) {
            return Ok(());
        }
        retain_root(&mut self.pools, new_root)?;
        #[cfg(verus_only)]
        proof_decl! { let ghost claimed = crate::state::pools_view(&self.pools); }
        #[cfg(verus_only)]
        proof! { proof::active_contains(&before.roots, root, old_root); }
        self.roots.replace(root, new_root);
        if let Child::Ptr(idx) = old_root {
            #[cfg(verus_only)]
            proof! { proof::claim_eligible(&before, &self.pools, old_root, new_root, idx); }
            crate::rc::rc_dec_or_remove(&mut self.pools, idx, ROOT_LOD, &mut self.scratch_stack)?;
        }
        #[cfg(verus_only)]
        proof! { proof::publish_committed(&before, self, root, old_root, new_root, claimed); }
        Ok(())
    }
}

/// 根发布前认领一份真实节点持有；调用方先验证 Child 的所属层与存活状态。
#[cfg_attr(verus_only, verus_verify)]
#[cfg_attr(verus_only, verus_spec(result =>
    requires crate::state::well_formed(old(pools)),
        crate::state::all_occupied_live(crate::state::pools_view(old(pools))),
        crate::state::child_valid(crate::state::pools_view(old(pools)), child, 10),
    ensures crate::state::well_formed(final(pools)),
        crate::state::all_occupied_live(crate::state::pools_view(final(pools))),
        result is Err ==> crate::state::pools_view(final(pools)) == crate::state::pools_view(old(pools)),
        result is Ok ==> proof::credited(crate::state::pools_view(old(pools)), crate::state::pools_view(final(pools)), child),
))]
#[inline(always)]
// 失败返回前需连接真实池状态的擦除证明。
#[allow(clippy::question_mark)]
fn retain_root(pools: &mut [Pool; DAG_DEPTH as usize], child: Child) -> Result<(), Error> {
    #[cfg(verus_only)]
    proof_decl! { let ghost before = crate::state::pools_view(pools); }
    if let Child::Ptr(index) = child {
        let pool = &mut pools[9];
        #[cfg(verus_only)]
        proof! { pool.live_model(crate::node::proof::idx(index)); }
        let result = pool.rc_inc(index);
        #[cfg(verus_only)]
        proof! { pool.valid_model(); }
        if let Err(error) = result {
            #[cfg(verus_only)]
            proof! { assert(crate::state::pools_view(pools) =~= before); }
            return Err(error);
        }
    }
    #[cfg(verus_only)]
    proof! { proof::root_credit(before, crate::state::pools_view(pools), child); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ZOrder20x3;
    use crate::read::SubtreeState;
    use crate::write::fill_uniform::UniformFill;
    use crate::write::set_cells::SetCell;
    use hashbrown::HashMap;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    use crate::test_util::{self, Point};

    const REF_RANGE: u32 = 64;
    const REF_MAX_LOD: u8 = 4;

    fn new_dag() -> HashDag {
        let result = HashDag::new();
        assert!(result.is_ok());
        result.unwrap_or_else(|_| std::process::abort())
    }

    #[derive(Clone, Debug, Default)]
    struct RefWorld {
        cells: HashMap<Point, u32>,
    }

    impl RefWorld {
        fn new() -> Self {
            Self::default()
        }
        fn value_at(&self, point: Point) -> u32 {
            self.cells.get(&point).copied().unwrap_or(0)
        }
        fn set_cell(&mut self, point: Point, data: u32) {
            if data == 0 {
                self.cells.remove(&point);
            } else {
                self.cells.insert(point, data);
            }
        }
        fn fill(&mut self, point: Point, lod: u8, data: u32) {
            let base = point.aligned_to(lod);
            let size = 1u32 << lod;
            for x in base.x..base.x + size {
                for y in base.y..base.y + size {
                    for z in base.z..base.z + size {
                        self.set_cell(Point::new(x, y, z), data);
                    }
                }
            }
        }
        fn sample_points(&self) -> Vec<Point> {
            let last = ZOrder20x3::RANGE_END - 1;
            let mut points: Vec<Point> = self.cells.keys().copied().collect();
            points.extend([
                Point::new(0, 0, 0),
                Point::new(1, 2, 3),
                Point::new(REF_RANGE - 1, REF_RANGE - 1, REF_RANGE - 1),
                Point::new(last, 0, 0),
                Point::new(last, last, last),
            ]);
            points.sort_unstable();
            points.dedup();
            points
        }
        fn subtree_state(&self, point: Point, lod: u8) -> SubtreeState {
            let base = point.aligned_to(lod);
            let size = 1u32 << lod;
            let mut first = None;
            let mut uniform = true;
            for x in base.x..base.x + size {
                for y in base.y..base.y + size {
                    for z in base.z..base.z + size {
                        let value = self.value_at(Point::new(x, y, z));
                        if let Some(first_value) = first {
                            uniform &= first_value == value;
                        } else {
                            first = Some(value);
                        }
                    }
                }
            }
            if uniform {
                SubtreeState::Uniform {
                    value: first.unwrap_or(0),
                }
            } else {
                SubtreeState::Mixed
            }
        }
    }

    // -- proptest helpers --

    #[derive(Clone, Debug)]
    struct TrackedRoot {
        id: RootId,
        world: RefWorld,
    }

    #[derive(Clone, Copy, Debug)]
    enum ApiOp {
        Fork {
            root_key: usize,
        },
        Release {
            root_key: usize,
        },
        Set {
            root_key: usize,
            point: Point,
            data: u32,
        },
        Fill {
            root_key: usize,
            point: Point,
            lod: u8,
            data: u32,
        },
    }

    #[derive(Clone, Copy, Debug)]
    enum WriteOp {
        Set { point: Point, data: u32 },
        Fill { point: Point, lod: u8, data: u32 },
    }

    fn svdag_ok<T>(result: Result<T, Error>) -> Result<T, TestCaseError> {
        result.map_err(|error| TestCaseError::fail(format!("unexpected svdag error: {error:?}")))
    }

    fn nth_active_root(roots: &[Option<TrackedRoot>], key: usize) -> Option<usize> {
        let active_count = roots.iter().filter(|root| root.is_some()).count();
        if active_count == 0 {
            return None;
        }
        let mut remaining = key % active_count;
        for (idx, root) in roots.iter().enumerate() {
            if root.is_none() {
                continue;
            }
            if remaining == 0 {
                return Some(idx);
            }
            remaining -= 1;
        }
        None
    }

    fn nth_releasable_root(roots: &[Option<TrackedRoot>], key: usize) -> Option<usize> {
        let active_count = roots
            .iter()
            .enumerate()
            .filter(|(idx, root)| *idx != 0 && root.is_some())
            .count();
        if active_count == 0 {
            return None;
        }
        let mut remaining = key % active_count;
        for (idx, root) in roots.iter().enumerate() {
            if idx == 0 || root.is_none() {
                continue;
            }
            if remaining == 0 {
                return Some(idx);
            }
            remaining -= 1;
        }
        None
    }

    fn assert_root_matches_reference(
        dag: &HashDag,
        root: &TrackedRoot,
    ) -> Result<(), TestCaseError> {
        for point in root.world.sample_points() {
            let value = svdag_ok(dag.value_at(root.id, point.path()))?;
            prop_assert_eq!(value, root.world.value_at(point));
        }
        Ok(())
    }

    fn assert_stale_roots_rejected(
        dag: &HashDag,
        stale_roots: &[RootId],
    ) -> Result<(), TestCaseError> {
        for root in stale_roots {
            prop_assert!(
                matches!(
                    dag.value_at(*root, test_util::path(0, 0, 0)),
                    Err(Error::RootMissing)
                ),
                "unexpected result"
            );
        }
        Ok(())
    }

    fn arb_ref_point() -> impl Strategy<Value = Point> {
        (0u32..REF_RANGE, 0u32..REF_RANGE, 0u32..REF_RANGE)
            .prop_map(|(x, y, z)| Point::new(x, y, z))
    }

    fn arb_boundary_point() -> impl Strategy<Value = Point> {
        let base = ZOrder20x3::RANGE_END - REF_RANGE;
        (0u32..REF_RANGE, 0u32..REF_RANGE, 0u32..REF_RANGE)
            .prop_map(move |(x, y, z)| Point::new(base + x, base + y, base + z))
    }

    fn arb_api_op() -> impl Strategy<Value = ApiOp> {
        prop_oneof![
            2 => (0usize..16usize).prop_map(|root_key| ApiOp::Fork { root_key }),
            2 => (0usize..16usize).prop_map(|root_key| ApiOp::Release { root_key }),
            5 => (0usize..16usize, arb_ref_point(), 0u32..8u32)
                .prop_map(|(root_key, point, data)| ApiOp::Set { root_key, point, data }),
            4 => (0usize..16usize, arb_ref_point(), 0u8..=3u8, 0u32..8u32)
                .prop_map(|(root_key, point, lod, data)| ApiOp::Fill { root_key, point, lod, data }),
        ]
    }

    fn arb_write_op() -> impl Strategy<Value = WriteOp> {
        prop_oneof![
            (arb_ref_point(), 0u32..8u32).prop_map(|(point, data)| WriteOp::Set { point, data }),
            (arb_ref_point(), 0u8..=REF_MAX_LOD, 0u32..8u32)
                .prop_map(|(point, lod, data)| WriteOp::Fill { point, lod, data }),
        ]
    }

    #[test]
    fn new_dag_value_at_is_zero() {
        let dag = new_dag();
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(0, 0, 0))).ok(),
            Some(0)
        );
    }

    #[test]
    fn new_dag_root_child_is_leaf_zero() {
        let dag = new_dag();
        assert_eq!((dag.root_child(dag.root())).ok(), Some(Child::Leaf(0)));
    }

    #[test]
    fn release_default_root_errors() {
        let mut dag = new_dag();
        assert!(
            matches!(
                dag.release_root(dag.root()),
                Err(Error::CannotReleaseDefaultRoot)
            ),
            "unexpected result"
        );
    }

    #[test]
    fn foreign_roots_do_not_fork_or_release_local_nodes() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let other = HashDag::new()?;
        let root = dag.fork_root(dag.root())?;
        let foreign = other.root();
        assert!(matches!(dag.fork_root(foreign), Err(Error::WrongOwner)));
        assert!(matches!(dag.release_root(foreign), Err(Error::WrongOwner)));
        assert_eq!(dag.root_child(root)?, Child::Leaf(0));
        dag.release_root(root)
    }

    #[test]
    fn allocation_error_preserves_standard_source() {
        let error = Vec::<u8>::new()
            .try_reserve(usize::MAX)
            .expect_err("impossible capacity");
        let error = Error::from(error);
        assert!(
            std::error::Error::source(&error)
                .is_some_and(|source| { source.is::<std::collections::TryReserveError>() })
        );
    }

    #[test]
    fn hash_allocation_error_preserves_hashbrown_source() {
        let error = hashbrown::HashTable::<u32>::new()
            .try_reserve(usize::MAX, |value| u64::from(*value))
            .expect_err("impossible capacity");
        let error = Error::from(error);
        assert!(
            std::error::Error::source(&error)
                .is_some_and(|source| { source.is::<hashbrown::TryReserveError>() })
        );
    }

    #[test]
    fn failed_fork_does_not_allocate_a_live_root() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        dag.set_cells(
            root,
            &[SetCell {
                path: ZOrder20x3::ZERO,
                data: 7,
            }],
        )?;
        let Child::Ptr(index) = dag.root_child(root)? else {
            return Err(Error::BuildFrameMissing);
        };
        let pool = &mut dag.pools[usize::from(ROOT_LOD)];
        let entry = pool.get(index).ok_or(Error::NodeMissing(index))?.to_entry();
        let _ = pool.rc_add_or_insert(usize::MAX - 1, &entry)?;
        let root_count = dag.roots.slots.len();
        assert!(matches!(dag.fork_root(root), Err(Error::RefCountOverflow)));
        assert_eq!(dag.roots.slots.len(), root_count);
        assert!(dag.roots.free.is_empty());
        assert_eq!(
            dag.pools[usize::from(ROOT_LOD)].ref_count(index),
            Some(usize::MAX)
        );
        Ok(())
    }

    #[test]
    fn saturated_release_preserves_snapshot_ownership() -> Result<(), TestCaseError> {
        let mut dag = svdag_ok(HashDag::new())?;
        let root = dag.root();
        let path = ZOrder20x3::ZERO;
        svdag_ok(dag.set_cells(root, &[SetCell { path, data: 7 }]))?;
        let mut snapshot = svdag_ok(dag.fork_root(root))?;
        svdag_ok(dag.fill_uniform(
            root,
            &[UniformFill {
                path,
                lod: HashDag::AXIS_BITS,
                data: 0,
            }],
        ))?;
        let child = svdag_ok(dag.root_child(snapshot))?;
        let Child::Ptr(index) = child else {
            return Err(TestCaseError::fail("snapshot root is not a node"));
        };
        prop_assert_eq!(
            svdag_ok(
                dag.pools
                    .get(usize::from(ROOT_LOD))
                    .ok_or(Error::InvalidLod(ROOT_LOD))
            )?
            .ref_count(index),
            Some(1)
        );
        svdag_ok(
            dag.roots
                .slots
                .get_mut(snapshot.slot)
                .ok_or(Error::RootMissing),
        )?
        .generation = u64::MAX;
        snapshot.generation = u64::MAX;
        let lengths = dag.pools.each_ref().map(Pool::len);

        prop_assert!(
            matches!(
                dag.release_root(snapshot),
                Err(Error::RootGenerationExhausted)
            ),
            "unexpected result"
        );
        prop_assert_eq!(svdag_ok(dag.root_child(snapshot))?, child);
        prop_assert_eq!(
            svdag_ok(
                dag.pools
                    .get(usize::from(ROOT_LOD))
                    .ok_or(Error::InvalidLod(ROOT_LOD))
            )?
            .ref_count(index),
            Some(1)
        );
        prop_assert_eq!(dag.pools.each_ref().map(Pool::len), lengths);
        prop_assert!(!dag.roots.free.contains(&snapshot.slot));
        svdag_ok(dag.set_cells(root, &[SetCell { path, data: 9 }]))?;
        prop_assert_eq!(svdag_ok(dag.value_at(snapshot, path))?, 7);
        prop_assert_eq!(svdag_ok(dag.value_at(root, path))?, 9);
        Ok(())
    }

    #[test]
    fn fork_creates_independent_snapshot() {
        let mut dag = new_dag();
        let Some(fork) = test_util::ok(dag.fork_root(dag.root())) else {
            return;
        };
        let _ = test_util::ok(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: test_util::path(1, 0, 0),
                data: 1,
            }],
        ));
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(1, 0, 0))).ok(),
            Some(1)
        );
        assert_eq!((dag.value_at(fork, test_util::path(1, 0, 0))).ok(), Some(0));
        let _ = test_util::ok(dag.release_root(fork));
    }

    #[test]
    fn stale_root_after_release_is_rejected() {
        let mut dag = new_dag();
        let Some(fork) = test_util::ok(dag.fork_root(dag.root())) else {
            return;
        };
        let _ = test_util::ok(dag.release_root(fork));
        assert!(
            matches!(
                dag.value_at(fork, test_util::path(0, 0, 0)),
                Err(Error::RootMissing)
            ),
            "unexpected result"
        );
    }

    #[test]
    fn set_cells_then_value_at_reads_data() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: test_util::path(7, 5, 3),
                data: 42,
            }],
        ));
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(7, 5, 3))).ok(),
            Some(42)
        );
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(7, 5, 4))).ok(),
            Some(0)
        );
    }

    #[test]
    fn fill_uniform_then_value_at_reads_fill_inside_and_outside() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: test_util::path(64, 0, 0),
                lod: 6,
                data: 3,
            }],
        ));
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(64, 0, 0))).ok(),
            Some(3)
        );
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(127, 63, 63))).ok(),
            Some(3)
        );
        assert_eq!(
            (dag.value_at(dag.root(), test_util::path(63, 0, 0))).ok(),
            Some(0)
        );
    }

    #[test]
    fn fill_at_root_lod_returns_leaf() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: ZOrder20x3::ZERO,
                lod: HashDag::AXIS_BITS,
                data: 7,
            }],
        ));
        assert_eq!((dag.root_child(dag.root())).ok(), Some(Child::Leaf(7)));
    }

    #[test]
    fn fill_then_set_cells_no_leak() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: ZOrder20x3::ZERO,
                lod: 3,
                data: 7,
            }],
        ));
        let _ = test_util::ok(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: ZOrder20x3::ZERO,
                data: 9,
            }],
        ));
    }

    #[test]
    fn subtree_at_empty_dag_is_uniform_zero() {
        let dag = new_dag();
        assert_eq!(
            (dag.subtree_at(dag.root(), test_util::path(0, 0, 0), DAG_DEPTH)).ok(),
            Some(SubtreeState::Uniform { value: 0 })
        );
    }

    #[test]
    fn subtree_at_after_set_is_mixed_then_uniform_at_leaf() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: test_util::path(1, 0, 0),
                data: 7,
            }],
        ));
        // 根区域包含不同值。
        assert_eq!(
            (dag.subtree_at(dag.root(), test_util::path(0, 0, 0), DAG_DEPTH)).ok(),
            Some(SubtreeState::Mixed)
        );
        // 命中体素的单元区域均匀。
        assert_eq!(
            (dag.subtree_at(dag.root(), test_util::path(1, 0, 0), 0)).ok(),
            Some(SubtreeState::Uniform { value: 7 })
        );
    }

    #[test]
    fn root_child_after_set_is_ptr() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: test_util::path(1, 0, 0),
                data: 1,
            }],
        ));
        assert!(matches!(dag.root_child(dag.root()), Ok(Child::Ptr(_))));
    }

    #[test]
    fn children_of_returns_all_sixty_four_slots() {
        let mut dag = new_dag();
        // 在 (0,0,0)..(1,0,0) 设置不同值使 leaf 节点非 uniform
        for x in 0u32..8 {
            let _ = test_util::ok(dag.set_cells(
                dag.root(),
                &[SetCell {
                    path: test_util::path(x, 0, 0),
                    data: x + 1,
                }],
            ));
        }
        let Some(root) = test_util::ok(dag.root_child(dag.root())) else {
            return;
        };
        let Child::Ptr(idx) = root else {
            assert!(matches!(root, Child::Ptr(_)));
            return;
        };
        let Some(children) = test_util::ok(dag.children_of(ROOT_LOD, idx)) else {
            return;
        };
        assert_eq!(children.len(), 64);
        // slot 0 应是 mixed（含有不同值）
        assert!(matches!(children[0], Child::Ptr(_)));
        // slot 1-63 应是 uniform Leaf(0)（未写入的子树保持默认值）
        for child in children.iter().skip(1) {
            assert_eq!(*child, Child::Leaf(0));
        }
    }

    #[test]
    fn children_of_invalid_lod_errors() {
        let dag = new_dag();
        assert!(matches!(
            dag.children_of(DAG_DEPTH, NodeIdx::ZERO),
            Err(Error::InvalidLod(DAG_DEPTH))
        ));
    }

    #[test]
    fn traversal_walk_reaches_uniform_fill() {
        let mut dag = new_dag();
        let _ = test_util::ok(dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: test_util::path(0, 0, 0),
                lod: 6,
                data: 3,
            }],
        ));
        let Some(mut child) = test_util::ok(dag.root_child(dag.root())) else {
            return;
        };
        let mut lod = ROOT_LOD;
        loop {
            child = match child {
                Child::Leaf(value) => {
                    assert_eq!(value, 3);
                    break;
                }
                Child::Ptr(idx) => {
                    let Some(children) = test_util::ok(dag.children_of(lod - 1, idx)) else {
                        return;
                    };
                    lod -= 1;
                    children[0]
                }
            };
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn write_read_roundtrip_matches_reference_model(
            ops in prop::collection::vec(arb_write_op(), 0..24),
            queries in prop::collection::vec((arb_ref_point(), 0u8..=REF_MAX_LOD), 1..32),
        ) {
            let mut dag = svdag_ok(HashDag::new())?;
            let mut world = RefWorld::new();

            for op in ops {
                match op {
                    WriteOp::Set { point, data } => {
                        svdag_ok(dag.set_cells(dag.root(), &[SetCell {
                            path: point.path(), data,
                        }]))?;
                        world.set_cell(point, data);
                    }
                    WriteOp::Fill { point, lod, data } => {
                        svdag_ok(dag.fill_uniform(dag.root(), &[UniformFill {
                            path: point.path(), lod, data,
                        }]))?;
                        world.fill(point, lod, data);
                    }
                }
            }

            // value_at 与 subtree_at(lod=0) 一致
            for (point, lod) in queries {
                prop_assert_eq!(
                    svdag_ok(dag.value_at(dag.root(), point.path()))?,
                    world.value_at(point)
                );
                prop_assert_eq!(
                    svdag_ok(dag.subtree_at(dag.root(), point.path(), lod))?,
                    world.subtree_state(point, lod)
                );
            }
        }

        #[test]
        fn snapshot_isolation_under_mixed_operations(
            ops in prop::collection::vec(arb_api_op(), 1..32),
        ) {
            let mut dag = svdag_ok(HashDag::new())?;
            let mut roots = vec![Some(TrackedRoot {
                id: dag.root(),
                world: RefWorld::new(),
            })];
            let mut stale_roots = Vec::new();

            for op in ops {
                match op {
                    ApiOp::Fork { root_key } => {
                        let Some(idx) = nth_active_root(&roots, root_key) else { continue };
                        let Some(root) = roots.get(idx).and_then(Option::as_ref) else { continue };
                        let fork = svdag_ok(dag.fork_root(root.id))?;
                        roots.push(Some(TrackedRoot {
                            id: fork,
                            world: root.world.clone(),
                        }));
                    }
                    ApiOp::Release { root_key } => {
                        let Some(idx) = nth_releasable_root(&roots, root_key) else {
                            prop_assert!(matches!(dag.release_root(dag.root()), Err(Error::CannotReleaseDefaultRoot)), "unexpected result");
                            continue;
                        };
                        let Some(root) = roots.get_mut(idx).and_then(Option::take) else { continue };
                        prop_assert_eq!((dag.release_root(root.id)).ok(), Some(()));
                        stale_roots.push(root.id);
                    }
                    ApiOp::Set { root_key, point, data } => {
                        let Some(idx) = nth_active_root(&roots, root_key) else { continue };
                        let Some(root) = roots.get_mut(idx).and_then(Option::as_mut) else { continue };
                        svdag_ok(dag.set_cells(root.id, &[SetCell {
                            path: point.path(), data,
                        }]))?;
                        root.world.set_cell(point, data);
                    }
                    ApiOp::Fill { root_key, point, lod, data } => {
                        let Some(idx) = nth_active_root(&roots, root_key) else { continue };
                        let Some(root) = roots.get_mut(idx).and_then(Option::as_mut) else { continue };
                        svdag_ok(dag.fill_uniform(root.id, &[UniformFill {
                            path: point.path(), lod, data,
                        }]))?;
                        root.world.fill(point, lod, data);
                    }
                }
                assert_stale_roots_rejected(&dag, &stale_roots)?;
            }

            for root in roots.iter().flatten() {
                assert_root_matches_reference(&dag, root)?;
            }
            assert_stale_roots_rejected(&dag, &stale_roots)?;
        }

        #[test]
        fn slot_reuse_rejects_stale_generation(cycles in 1usize..32usize) {
            let mut dag = svdag_ok(HashDag::new())?;
            prop_assert!(matches!(dag.release_root(dag.root()), Err(Error::CannotReleaseDefaultRoot)), "unexpected result");
            for _ in 0..cycles {
                let stale = svdag_ok(dag.fork_root(dag.root()))?;
                svdag_ok(dag.release_root(stale))?;
                let fresh = svdag_ok(dag.fork_root(dag.root()))?;
                prop_assert!(matches!(dag.value_at(stale, test_util::path(0, 0, 0)), Err(Error::RootMissing)), "unexpected result");
                prop_assert_eq!((dag.value_at(fresh, test_util::path(0, 0, 0))).ok(), Some(0));
                svdag_ok(dag.release_root(fresh))?;
            }
        }

        #[test]
        fn boundary_cells_use_full_root_range(
            writes in prop::collection::vec((arb_boundary_point(), 1u32..8u32), 1..24),
        ) {
            let mut dag = svdag_ok(HashDag::new())?;
            let fork = svdag_ok(dag.fork_root(dag.root()))?;
            let mut world = RefWorld::new();
            for (point, data) in writes {
                world.set_cell(point, data);
            }
            let mut cells: Vec<SetCell> = world.cells.iter()
                .map(|(point, data)| SetCell { path: point.path(), data: *data })
                .collect();
            cells.sort_by_key(|cell| cell.path.value());
            svdag_ok(dag.set_cells(dag.root(), &cells))?;
            for point in world.sample_points() {
                let expected = world.value_at(point);
                prop_assert_eq!((dag.value_at(dag.root(), point.path())).ok(), Some(expected));
                prop_assert_eq!((dag.value_at(fork, point.path())).ok(), Some(0));
            }
            svdag_ok(dag.release_root(fork))?;
        }

        #[test]
        fn invalid_lod_is_rejected(lod in (HashDag::AXIS_BITS + 1)..=u8::MAX) {
            let mut dag = svdag_ok(HashDag::new())?;
            let p = test_util::path(0, 0, 0);
            prop_assert!(matches!(dag.fill_uniform(dag.root(), &[UniformFill { path: p, lod, data: 1 }]), Err(Error::InvalidLod(actual)) if actual == lod), "unexpected result");
            prop_assert!(matches!(dag.subtree_at(dag.root(), p, lod), Err(Error::InvalidLod(actual)) if actual == lod), "unexpected result");
        }
    }
}

#[cfg(test)]
mod native_tests {
    use crate::{
        Child, DenseRegion, Error, HashDag, SetCell, SubtreeState, UniformFill, ZOrder20x3,
    };

    #[test]
    fn every_child_at_every_level() -> Result<(), Error> {
        for level in 0..HashDag::DEPTH {
            let mut dag = HashDag::new()?;
            let cells = (0..64_u32)
                .map(|slot| {
                    Ok(SetCell {
                        path: ZOrder20x3::from_value(u64::from(slot) << (6 * level))
                            .ok_or(Error::InvalidInput)?,
                        data: u32::MAX - slot,
                    })
                })
                .collect::<Result<Vec<_>, Error>>()?;
            dag.set_cells(dag.root(), &cells)?;
            for cell in &cells {
                if dag.value_at(dag.root(), cell.path)? != cell.data {
                    return Err(Error::InvalidInput);
                }
            }
            let Child::Ptr(node) =
                dag.subtree_child(dag.root(), ZOrder20x3::ZERO, (level + 1) * 2)?
            else {
                return Err(Error::InvalidInput);
            };
            let children = dag.children_of(level, node)?;
            if children.len() != 64 {
                return Err(Error::InvalidInput);
            }
            for (slot, child) in children.into_iter().enumerate() {
                if child
                    != dag.child_of(
                        level,
                        node,
                        u8::try_from(slot).map_err(|_| Error::InvalidInput)?,
                    )?
                    || matches!(child, Child::Ptr(_)) != (level != 0)
                {
                    return Err(Error::InvalidInput);
                }
            }
            let (values, masks) = dag.node_arrays(level)?;
            if values.len() != masks.len()
                || values.as_ptr().addr() % 64 != 0
                || masks.as_ptr().addr() % 8 != 0
            {
                return Err(Error::InvalidInput);
            }
        }
        Ok(())
    }

    #[test]
    fn every_spatial_lod_fill_keeps_exact_extent() -> Result<(), Error> {
        for lod in 0..=HashDag::AXIS_BITS {
            let mut dag = HashDag::new()?;
            dag.fill_uniform(
                dag.root(),
                &[UniformFill {
                    path: ZOrder20x3::ZERO,
                    lod,
                    data: u32::MAX,
                }],
            )?;
            let edge = 1_u32 << lod;
            for x in [0, edge - 1] {
                for y in [0, edge - 1] {
                    for z in [0, edge - 1] {
                        let path = ZOrder20x3::encode(x, y, z).ok_or(Error::InvalidInput)?;
                        if dag.value_at(dag.root(), path)? != u32::MAX {
                            return Err(Error::InvalidInput);
                        }
                        if dag.subtree_at(dag.root(), path, lod)?
                            != (SubtreeState::Uniform { value: u32::MAX })
                        {
                            return Err(Error::InvalidInput);
                        }
                    }
                }
            }
            if lod < HashDag::AXIS_BITS {
                for point in [(edge, 0, 0), (0, edge, 0), (0, 0, edge)] {
                    let path =
                        ZOrder20x3::encode(point.0, point.1, point.2).ok_or(Error::InvalidInput)?;
                    if dag.value_at(dag.root(), path)? != 0 {
                        return Err(Error::InvalidInput);
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn odd_and_even_dense_lods_match_morton_values() -> Result<(), Error> {
        for lod in 0..=5_u8 {
            let mut dag = HashDag::new()?;
            let values = (0..(1_u32 << (3 * lod)))
                .map(|value| u32::MAX - value)
                .collect::<Vec<_>>();
            dag.set_dense_regions(
                dag.root(),
                &[DenseRegion {
                    path: ZOrder20x3::ZERO,
                    lod,
                    values: &values,
                }],
            )?;
            for (code, &value) in values.iter().enumerate() {
                let path =
                    ZOrder20x3::from_value(u64::try_from(code).map_err(|_| Error::InvalidInput)?)
                        .ok_or(Error::InvalidInput)?;
                if dag.value_at(dag.root(), path)? != value {
                    return Err(Error::InvalidInput);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn overlapping_fills_fail_before_mutation() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let fills = [
            UniformFill {
                path: ZOrder20x3::ZERO,
                lod: 3,
                data: 7,
            },
            UniformFill {
                path: ZOrder20x3::ZERO,
                lod: 0,
                data: 8,
            },
        ];
        if !matches!(
            dag.fill_uniform(dag.root(), &fills),
            Err(Error::InvalidInput)
        ) {
            return Err(Error::InvalidInput);
        }
        if dag.root_child(dag.root())? != Child::Leaf(0) {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}

#[cfg(test)]
mod canonical_tests {
    use proptest::prelude::*;

    use crate::{Child, DenseRegion, HashDag, NodeIdx, RootId, SetCell, UniformFill, ZOrder20x3};

    fn checked<T>(result: Result<T, crate::Error>) -> Result<T, String> {
        result.map_err(|error| format!("unexpected svdag error: {error:?}"))
    }

    fn node_index(slot: usize) -> Result<NodeIdx, String> {
        u64::try_from(slot)
            .ok()
            .and_then(NodeIdx::new)
            .ok_or_else(|| format!("invalid node index: {slot}"))
    }

    fn path(code: u64) -> Result<ZOrder20x3, String> {
        ZOrder20x3::from_value(code).ok_or_else(|| format!("invalid Morton code: {code}"))
    }

    // 完整比较各层 payload 和 mask，不复用生产哈希表或折叠检测函数。
    fn audit_canonical_nodes(dag: &HashDag) -> Result<(), String> {
        for (level, pool) in dag.pools.iter().enumerate() {
            for slot in 0..pool.slot_count() {
                let Some(node) = pool.get(node_index(slot)?) else {
                    continue;
                };
                let first = node.child(0);
                if matches!(first, Child::Leaf(_))
                    && (1..64_u8).all(|child| node.child(child) == first)
                {
                    return Err(format!(
                        "unfolded uniform node at level {level}, slot {slot}"
                    ));
                }
                for previous in 0..slot {
                    if let Some(other) = pool.get(node_index(previous)?)
                        && node.mask == other.mask
                        && node.values == other.values
                    {
                        return Err(format!(
                            "duplicate nodes at level {level}, slots {previous} and {slot}"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn audit_references(dag: &HashDag) -> Result<(), String> {
        let mut expected = dag
            .pools
            .iter()
            .map(|pool| vec![0_usize; pool.slot_count()])
            .collect::<Vec<_>>();
        for (level, pool) in dag.pools.iter().enumerate() {
            for slot in 0..pool.slot_count() {
                let Some(node) = pool.get(node_index(slot)?) else {
                    continue;
                };
                for child in 0..64_u8 {
                    if let Child::Ptr(index) = node.child(child) {
                        let child_level = level
                            .checked_sub(1)
                            .ok_or_else(|| format!("pointer below level zero at slot {slot}"))?;
                        let child_slot = checked(index.as_usize())?;
                        let count = expected
                            .get_mut(child_level)
                            .and_then(|counts| counts.get_mut(child_slot))
                            .ok_or_else(|| {
                                format!("missing child at level {level}, slot {slot}")
                            })?;
                        *count += 1;
                    }
                }
            }
        }
        for (slot, root) in dag.roots.slots.iter().enumerate() {
            if let Some(Child::Ptr(index)) = root.child {
                let count = expected
                    .last_mut()
                    .and_then(|counts| counts.get_mut(index.as_usize().ok()?))
                    .ok_or_else(|| format!("missing root node at root slot {slot}"))?;
                *count += 1;
            }
        }
        for (level, (pool, counts)) in dag.pools.iter().zip(expected).enumerate() {
            for (slot, count) in counts.into_iter().enumerate() {
                let actual = pool.ref_count(node_index(slot)?);
                if actual != Some(count) {
                    return Err(format!(
                        "reference mismatch at level {level}, slot {slot}: actual {actual:?}, expected {count}"
                    ));
                }
            }
        }
        Ok(())
    }

    fn audit(dag: &HashDag) -> Result<(), String> {
        audit_canonical_nodes(dag)?;
        audit_references(dag)
    }

    fn verify_values(
        dag: &HashDag,
        root: RootId,
        start: u64,
        values: &[u32],
    ) -> Result<(), String> {
        for (offset, &expected) in values.iter().enumerate() {
            let offset = u64::try_from(offset).map_err(|error| error.to_string())?;
            let actual = checked(dag.value_at(root, path(start + offset)?))?;
            if actual != expected {
                return Err(format!(
                    "value mismatch at Morton code {}: actual {actual}, expected {expected}",
                    start + offset
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn dense_nonadjacent_repeated_patterns_are_unique() -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        let values = (0..4096_u32)
            .map(|index| 1 + (index % 64) % 7 + ((index / 64) % 2) * 16)
            .collect::<Vec<_>>();
        checked(dag.set_dense_regions(
            dag.root(),
            &[DenseRegion {
                path: ZOrder20x3::ZERO,
                lod: 4,
                values: &values,
            }],
        ))?;
        verify_values(&dag, dag.root(), 0, &values)?;
        audit(&dag)
    }

    #[test]
    fn repeated_dense_patterns_across_chunks_are_unique() -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        let a = std::array::from_fn::<_, 64, _>(|index| u32::from(index % 2 == 0));
        let b = std::array::from_fn::<_, 64, _>(|index| 2 + u32::from(index % 3 == 0));
        let chunks = [
            DenseRegion {
                path: path(0)?,
                lod: 2,
                values: &a,
            },
            DenseRegion {
                path: path(64)?,
                lod: 2,
                values: &b,
            },
            DenseRegion {
                path: path(128)?,
                lod: 2,
                values: &a,
            },
            DenseRegion {
                path: path(192)?,
                lod: 2,
                values: &b,
            },
        ];
        checked(dag.set_dense_regions(dag.root(), &chunks))?;
        for chunk in &chunks {
            verify_values(&dag, dag.root(), chunk.path.value(), chunk.values)?;
        }
        audit(&dag)
    }

    #[test]
    fn set_cells_nonadjacent_repeated_patterns_are_unique() -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        let cells = [
            SetCell {
                path: path(0)?,
                data: 7,
            },
            SetCell {
                path: path(64)?,
                data: 11,
            },
            SetCell {
                path: path(128)?,
                data: 7,
            },
            SetCell {
                path: path(192)?,
                data: 11,
            },
        ];
        checked(dag.set_cells(dag.root(), &cells))?;
        for cell in cells {
            verify_values(&dag, dag.root(), cell.path.value(), &[cell.data, 0])?;
        }
        audit(&dag)
    }

    #[test]
    fn fills_nonadjacent_repeated_patterns_are_unique() -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        let fills = [
            UniformFill {
                path: path(0)?,
                lod: 2,
                data: 7,
            },
            UniformFill {
                path: path(4096)?,
                lod: 2,
                data: 11,
            },
            UniformFill {
                path: path(8192)?,
                lod: 2,
                data: 7,
            },
            UniformFill {
                path: path(12288)?,
                lod: 2,
                data: 11,
            },
        ];
        checked(dag.fill_uniform(dag.root(), &fills))?;
        for fill in fills {
            verify_values(&dag, dag.root(), fill.path.value(), &[fill.data; 64])?;
            verify_values(&dag, dag.root(), fill.path.value() + 64, &[0])?;
        }
        audit(&dag)
    }

    #[test]
    fn shared_mixed_children_keep_their_spatial_level() -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        let values = (0..4096_u32).map(|index| index % 64).collect::<Vec<_>>();
        checked(dag.set_dense_regions(
            dag.root(),
            &[DenseRegion {
                path: path(0)?,
                lod: 4,
                values: &values,
            }],
        ))?;
        let Child::Ptr(parent) = checked(dag.subtree_child(dag.root(), path(0)?, 4))? else {
            return Err("mixed parent was collapsed to a leaf".to_owned());
        };
        let children = checked(dag.children_of(1, parent))?;
        if !matches!(children[0], Child::Ptr(_))
            || children.iter().any(|&child| child != children[0])
        {
            return Err("repeated mixed children do not share one node".to_owned());
        }
        verify_values(&dag, dag.root(), 0, &values)?;
        audit(&dag)
    }

    #[derive(Clone, Copy)]
    enum Restore {
        Cells,
        Fills,
        Dense,
    }

    fn restored_uniform_root(method: Restore) -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        checked(dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: path(0)?,
                lod: HashDag::AXIS_BITS,
                data: 9,
            }],
        ))?;
        let paths = [path(0)?, path(1_u64 << 54)?];
        let changed = paths.map(|path| SetCell { path, data: 7 });
        checked(dag.set_cells(dag.root(), &changed))?;
        audit(&dag)?;
        match method {
            Restore::Cells => {
                checked(dag.set_cells(dag.root(), &paths.map(|path| SetCell { path, data: 9 })))?;
            }
            Restore::Fills => {
                checked(dag.fill_uniform(
                    dag.root(),
                    &paths.map(|path| UniformFill {
                        path,
                        lod: 2,
                        data: 9,
                    }),
                ))?;
            }
            Restore::Dense => {
                let values = [9; 64];
                checked(dag.set_dense_regions(
                    dag.root(),
                    &paths.map(|path| DenseRegion {
                        path,
                        lod: 2,
                        values: &values,
                    }),
                ))?;
            }
        }
        if checked(dag.root_child(dag.root()))? != Child::Leaf(9) {
            return Err("restored uniform value did not fold through the root".to_owned());
        }
        if dag.pools.iter().any(|pool| pool.len() != 0) {
            return Err("restoring the uniform root retained live nodes".to_owned());
        }
        audit(&dag)
    }

    #[test]
    fn set_cells_restoration_folds_through_root() -> Result<(), String> {
        restored_uniform_root(Restore::Cells)
    }

    #[test]
    fn fill_restoration_folds_through_root() -> Result<(), String> {
        restored_uniform_root(Restore::Fills)
    }

    #[test]
    fn dense_restoration_folds_through_root() -> Result<(), String> {
        restored_uniform_root(Restore::Dense)
    }

    #[test]
    fn repeated_builds_and_retained_roots_share_and_reclaim_nodes() -> Result<(), String> {
        let mut dag = checked(HashDag::new())?;
        let other = checked(dag.fork_root(dag.root()))?;
        let values = (0..4096_u32)
            .map(|index| (index % 64) % 3 + ((index / 64) % 2) * 4)
            .collect::<Vec<_>>();
        let chunks = [DenseRegion {
            path: path(0)?,
            lod: 4,
            values: &values,
        }];
        checked(dag.set_dense_regions(dag.root(), &chunks))?;
        let original = checked(dag.root_child(dag.root()))?;
        for _ in 0..4 {
            checked(dag.set_dense_regions(other, &chunks))?;
            if checked(dag.root_child(other))? != original {
                return Err("equal roots do not share the canonical node".to_owned());
            }
            audit(&dag)?;
        }
        let retained = checked(dag.fork_root(other))?;
        audit(&dag)?;
        checked(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: path(0)?,
                data: 99,
            }],
        ))?;
        verify_values(&dag, other, 0, &values)?;
        verify_values(&dag, retained, 0, &values)?;
        audit(&dag)?;
        checked(dag.set_cells(
            dag.root(),
            &[SetCell {
                path: path(0)?,
                data: *values.first().ok_or("missing uniform value")?,
            }],
        ))?;
        if checked(dag.root_child(dag.root()))? != original {
            return Err("restored root did not reuse its retained canonical node".to_owned());
        }
        audit(&dag)?;
        checked(dag.release_root(other))?;
        audit(&dag)?;
        checked(dag.release_root(retained))?;
        verify_values(&dag, dag.root(), 0, &values)?;
        audit(&dag)?;
        checked(dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: path(0)?,
                lod: HashDag::AXIS_BITS,
                data: 0,
            }],
        ))?;
        if dag.pools.iter().any(|pool| pool.len() != 0) {
            return Err("released roots left live nodes".to_owned());
        }
        audit(&dag)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn equal_contents_from_all_write_apis_have_one_root(
            values in prop::collection::vec(0_u32..5, 512),
        ) {
            let run = || -> Result<(), String> {
                let mut dag = checked(HashDag::new())?;
                let cells_root = checked(dag.fork_root(dag.root()))?;
                let fills_root = checked(dag.fork_root(dag.root()))?;
                let cells = values.iter().enumerate().map(|(index, &data)| {
                    let code = u64::try_from(index).map_err(|error| error.to_string())?;
                    Ok(SetCell { path: path(code)?, data })
                }).collect::<Result<Vec<_>, String>>()?;
                let fills = cells.iter().map(|cell| UniformFill {
                    path: cell.path, lod: 0, data: cell.data,
                }).collect::<Vec<_>>();
                checked(dag.set_dense_regions(dag.root(), &[DenseRegion {
                    path: path(0)?, lod: 3, values: &values,
                }]))?;
                checked(dag.set_cells(cells_root, &cells))?;
                checked(dag.fill_uniform(fills_root, &fills))?;
                let canonical = checked(dag.root_child(dag.root()))?;
                for root in [dag.root(), cells_root, fills_root] {
                    verify_values(&dag, root, 0, &values)?;
                    if checked(dag.root_child(root))? != canonical {
                        return Err("equal contents have different canonical roots".to_owned());
                    }
                }
                audit(&dag)
            };
            run().map_err(proptest::test_runner::TestCaseError::fail)?;
        }
    }
}
