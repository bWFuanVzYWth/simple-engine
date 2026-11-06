//! 区块、异步请求、可见覆盖与延迟释放的唯一所有者。

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod coverage_tests;
#[cfg(test)]
#[path = "publication_tests.rs"]
mod publication_tests;
#[cfg(test)]
#[path = "transfer_tests.rs"]
mod transfer_tests;

use crate::{Chunk, Config, Target, index::Index, planner::Schedule};

/// 带槽代号的区块句柄；槽复用不会使旧句柄重新有效。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Handle {
    pub(crate) index: usize,
    pub(crate) generation: u64,
}

impl Handle {
    /// 返回所属状态内的槽索引，可用于长度为配置容量的临时位图。
    ///
    /// 槽可复用；跨批缓存须同时保存完整句柄并校验代号，或在每批清空位图。
    /// 索引不延长节点有效期，也不能在另一份状态中解释。
    pub const fn slot(self) -> usize {
        self.index
    }
}

/// 一次构建请求的代号；相机移动不使它失效。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Ticket {
    handle: Handle,
    generation: u64,
}

/// 交给宿主异步构建的请求，不持有资源。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Build {
    /// 完成或失败时交回的请求代号。
    pub ticket: Ticket,
    /// 要构建的区块。
    pub chunk: Chunk,
}

/// 待宿主准备接缝并确认发布的覆盖变化。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Change {
    /// 同一区块的编辑结果替换，不改变覆盖 cut。
    Replace(Chunk),
    /// 启用一个此前没有覆盖的根区块。
    Activate(Chunk),
    /// 移除一个叶区块；设置目标后仅允许移除需求根域之外的叶。
    Remove(Chunk),
    /// 用八个子区块替换父区块。
    Split {
        /// 当前可见的父区块。
        parent: Chunk,
        /// 按 xyz 三位索引排列的子区块。
        children: [Chunk; 8],
    },
    /// 用父区块替换八个子区块。
    Merge {
        /// 即将可见的父区块。
        parent: Chunk,
        /// 当前可见的八个子区块。
        children: [Chunk; 8],
    },
}

/// 待发布变化的代号；关联节点在确认前保持锁定。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PatchToken {
    handle: Handle,
    generation: u64,
}

/// 不持有资源的宿主准备请求。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Patch {
    /// 准备成功后传给 [`State::publish`] 的代号。
    pub token: PatchToken,
    /// 接缝与可见集合须共同应用的变化。
    pub change: Change,
}

/// 状态转换失败；容量失败不会部分修改资源或拓扑。
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StateError {
    /// 节点槽已满。
    #[error("区块容量已满")]
    Capacity,
    /// 延迟释放队列没有可预留的位置。
    #[error("延迟释放容量已满")]
    RetirementFull,
    /// 句柄已释放或其槽已复用。
    #[error("区块句柄已过期")]
    StaleHandle,
    /// 请求已经完成、取消或被替代。
    #[error("构建请求已过期")]
    StaleTicket,
    /// 待确认变化已取消或发布。
    #[error("覆盖变化已过期")]
    StalePatch,
    /// 节点正在等待宿主确认。
    #[error("区块正在等待覆盖变化确认")]
    PendingPatch,
    /// 当前资源或拓扑不允许此操作。
    #[error("当前区块状态不允许此操作")]
    Transition,
    /// 父子坐标、LOD 或索引不合法。
    #[error("区块层级不合法")]
    Hierarchy,
    /// 单调代号已经用尽。
    #[error("状态代号已用尽")]
    GenerationExhausted,
}

/// 未被状态接受的完成结果；宿主仍拥有 `resource`。
#[derive(Debug, thiserror::Error)]
#[error("完成结果未被接受：{error}")]
pub struct Rejected<R> {
    /// 拒绝原因。
    #[source]
    pub error: StateError,
    /// 原样返还的资源，不会被状态隐式销毁。
    pub resource: R,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Topology {
    Detached,
    Leaf,
    Branch,
}

pub(crate) enum ResourcePhase<R> {
    Empty,
    Requested(Ticket),
    Ready(R),
    Visible(R),
    Refreshing { current: R, ticket: Ticket },
    Replacement { current: R, next: R },
}

pub(crate) struct Node<R> {
    pub(crate) chunk: Chunk,
    pub(crate) parent: Option<Handle>,
    pub(crate) children: [Option<Handle>; 8],
    pub(crate) topology: Topology,
    pub(crate) phase: ResourcePhase<R>,
    pub(crate) pending: Option<PatchToken>,
    pub(crate) invalidated: bool,
    patch: Option<Kind>,
}

struct Slot<R> {
    generation: u64,
    occupied_index: usize,
    node: Option<Node<R>>,
}

/// 帧外扩容失败；失败保留资源、身份、队列和逻辑容量，已成功预留的内存可保留。
#[derive(Debug, thiserror::Error)]
pub enum GrowthError {
    /// 请求容量不合法。
    #[error("区块扩容配置无效：{0}")]
    Config(#[from] crate::ConfigError),
    /// 分配失败；保留分配器提供的底层来源。
    #[error("区块扩容分配失败：{0}")]
    Allocation(#[from] std::collections::TryReserveError),
}

#[derive(Clone, Copy)]
enum Kind {
    Replace,
    Activate,
    Remove,
    Split,
    Merge,
}

impl Kind {
    const fn retired(self) -> usize {
        match self {
            Self::Activate => 0,
            Self::Remove | Self::Split | Self::Replace => 1,
            Self::Merge => 8,
        }
    }
}

#[derive(Clone, Copy)]
struct Pending {
    token: PatchToken,
    kind: Kind,
    children: [Option<Handle>; 8],
}

struct Retired<R> {
    resource: R,
    after: u64,
}

/// 区块状态与资源唯一所有者；帧内按当前容量工作，帧外可显式扩容。
///
/// 构造时预留全部槽和队列。资源不要求 `Clone`；取消过期请求、发布覆盖和
/// 延迟释放都通过显式转换完成。节点释放只处理一个无子节点的离散节点。
/// 析构直接销毁剩余资源，不等待外部任务或 GPU；宿主销毁状态前须结束这些访问。
pub struct State<R> {
    config: Config,
    slots: Vec<Slot<R>>,
    occupied: Vec<Handle>,
    index: Index,
    free: Vec<usize>,
    generation: u64,
    retired: Vec<Option<Retired<R>>>,
    retired_head: usize,
    retired_len: usize,
    retired_reserved: usize,
    pending_requests: usize,
    pending_patches: usize,
    pub(crate) coverage_version: u64,
    pub(crate) prepared_version: u64,
    pub(crate) target: Option<Target>,
    pub(crate) schedule: Schedule,
}

/// 整批校验后的独占发布阶段；借用原patch切片，不复制场景或分配暂存。
///
/// GPU提交成功后调用[`Self::commit`]；提交失败或取消时直接丢弃，覆盖和代号均不变。
pub struct PatchPublication<'a, 'p, R> {
    state: &'a mut State<R>,
    patches: &'p [Patch],
    generation: u64,
    last_use: u64,
}
impl<R> PatchPublication<'_, '_, R> {
    /// 发布整批已准备覆盖；不分配、不返回错误，不代替外部GPU完成证明。
    pub fn commit(self) {
        for (index, patch) in self.patches.iter().enumerate() {
            let pending = self
                .state
                .pending(patch.token)
                .expect("独占发布阶段保持家族锁和唯一token");
            let generation = self.generation + index as u64 + 1;
            self.state.generation = generation;
            self.state
                .publish_pending(pending, generation, self.last_use);
        }
    }
}

impl<R> State<R> {
    /// 在任何外部发布前校验整批patch和代号余量，成功后保持本状态独占到提交或取消。
    ///
    /// 新资源/接缝须已准备；last_use语义同[`Self::publish`]。空批合法。
    /// 逐项校验加O(B²)重复token比较，不分配；应用应按自身帧预算限制本批B。
    /// 有效不同token的家族由既有锁保证互不重叠，不能借伪造子节点token重复发布家族。
    /// # Errors
    /// 任一patch过期/失效、输入重复或代号不足时整批不修改覆盖、退役队列或代号。
    pub fn prepare_publication<'a, 'p>(
        &'a mut self,
        patches: &'p [Patch],
        last_use: u64,
    ) -> Result<PatchPublication<'a, 'p, R>, StateError> {
        for (index, patch) in patches.iter().enumerate() {
            self.validated_pending(patch.token)?;
            if patches[..index]
                .iter()
                .any(|previous| previous.token == patch.token)
            {
                return Err(StateError::Transition);
            }
        }
        let count = u64::try_from(patches.len()).map_err(|_| StateError::GenerationExhausted)?;
        self.generation
            .checked_add(count)
            .ok_or(StateError::GenerationExhausted)?;
        let generation = self.generation;
        Ok(PatchPublication {
            state: self,
            patches,
            generation,
            last_use,
        })
    }

    /// 预分配节点、调度与延迟释放容量，不创建区块或资源。
    pub fn new(config: Config) -> Self {
        let capacity = config.capacity();
        Self {
            config,
            slots: (0..capacity)
                .map(|_| Slot {
                    generation: 0,
                    occupied_index: 0,
                    node: None,
                })
                .collect(),
            occupied: Vec::with_capacity(capacity),
            index: Index::with_capacity(capacity),
            free: (0..capacity).rev().collect(),
            generation: 0,
            retired: (0..capacity).map(|_| None).collect(),
            retired_head: 0,
            retired_len: 0,
            retired_reserved: 0,
            pending_requests: 0,
            pending_patches: 0,
            coverage_version: 0,
            prepared_version: 0,
            target: None,
            schedule: Schedule::new(config),
        }
    }

    /// 返回当前配置；层数固定，容量仅由显式扩容改变。
    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// 帧外增加节点、索引、调度和退役容量，不缩容；较小或相同容量为无操作。
    ///
    /// 保留所有句柄、ticket、待发布家族、资源及退役完成值，不重建拓扑。
    /// 全部可失败分配先于逻辑变更；失败时原状态仍可规划，已预留内存允许保留。
    /// 扩容后无需重新枚举目标，先前等待容量的工作可继续。
    pub fn grow_capacity(&mut self, capacity: usize) -> Result<(), GrowthError> {
        let config = self.config.with_capacity(capacity)?;
        let old = self.config.capacity();
        if capacity <= old {
            return Ok(());
        }
        self.slots.try_reserve_exact(capacity - self.slots.len())?;
        self.occupied
            .try_reserve_exact(capacity - self.occupied.len())?;
        self.free.try_reserve_exact(capacity - self.free.len())?;
        self.retired
            .try_reserve_exact(capacity - self.retired.len())?;
        self.index.reserve_capacity(capacity)?;
        self.schedule.reserve_capacity(capacity)?;
        self.retired.rotate_left(self.retired_head);
        self.retired_head = 0;
        self.retired.resize_with(capacity, || None);
        self.slots.resize_with(capacity, || Slot {
            generation: 0,
            occupied_index: 0,
            node: None,
        });
        self.free.extend((old..capacity).rev());
        self.index.grow_capacity(capacity);
        self.schedule.grow_capacity(capacity);
        self.config = config;
        Ok(())
    }

    /// 更改半径或有限边界，保留资源与请求，后续规划按预算重新协调。
    ///
    /// 与下一次规划的最终目标合并后决定差分；重复配置不启动工作。
    /// 不能直接差分时丢弃旧几何游标，保留占用节点核对位置；配置稳定后完成全部协调。
    /// 层数或容量变化返回 [`crate::ConfigError::Storage`]，且状态保持原状。
    pub fn reconfigure(&mut self, config: Config) -> Result<(), crate::ConfigError> {
        if config.lod_count() != self.config.lod_count()
            || config.capacity() != self.config.capacity()
        {
            return Err(crate::ConfigError::Storage);
        }
        if config != self.config {
            self.schedule.reconfigure(self.config);
            self.config = config;
        }
        Ok(())
    }

    /// 更新有效半径；已有资源不被立即释放。
    pub fn set_radius(&mut self, radius: u32) -> Result<(), crate::ConfigError> {
        self.reconfigure(self.config.with_radius(radius)?)
    }

    /// 逐槽查看已保留节点，不扫描其他槽；空槽返回 `None`。
    pub fn handle_at(&self, index: usize) -> Option<Handle> {
        self.visit_handle(index)
    }

    /// 标记编辑失效；刷新请求与替换由后续规划预算控制。
    ///
    /// 在途旧结果不会覆盖较新的编辑；已准备家族的任一父/子节点失效，整项变化都须
    /// 撤销后重新准备，旧可见覆盖不变。宿主须对编辑影响的各LOD节点分别标记；
    /// 本方法不推断资源内部的源版本或传播到尚未被标记的其他节点。
    pub fn invalidate(&mut self, handle: Handle) -> Result<(), StateError> {
        let pending = self
            .node(handle)
            .ok_or(StateError::StaleHandle)?
            .pending
            .is_some();
        if pending {
            self.prepared_version = self.next_generation()?;
        }
        self.node_mut(handle)?.invalidated = true;
        self.wake(handle);
        Ok(())
    }

    /// 状态本身及全部预留缓冲的字节数，包含节点、索引、调度和退役容量。
    ///
    /// 按实际保留容量和 `R` 的内联大小计算；不包含分配器开销、`R` 自己的间接分配、
    /// 宿主在途构建或 [`crate::Plan`] 缓冲。空状态也保留这些容量，规划内不会增长。
    pub fn reserved_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.slots.capacity() * std::mem::size_of::<Slot<R>>()
            + self.free.capacity() * std::mem::size_of::<usize>()
            + self.occupied.capacity() * std::mem::size_of::<Handle>()
            + self.retired.capacity() * std::mem::size_of::<Option<Retired<R>>>()
            + self.index.reserved_bytes()
            + self.schedule.reserved_bytes()
    }

    /// 返回已占用的节点槽数，包含预取与等待释放的节点。
    pub const fn len(&self) -> usize {
        self.index.len()
    }

    /// 是否没有占用的节点槽。
    pub const fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// 按区块键查找当前句柄。
    pub fn find(&self, chunk: Chunk) -> Option<Handle> {
        self.index.get(&chunk).copied()
    }

    /// 借用全部已占用节点的身份，成本为 `O(len)`，不扫描空槽、不持有资源镜像。
    pub fn occupied(&self) -> impl ExactSizeIterator<Item = (Handle, Chunk)> + '_ {
        self.occupied
            .iter()
            .map(|&handle| (handle, self.node(handle).expect("占用表句柄有效").chunk))
    }

    pub(crate) fn occupied_handle(&self, index: usize) -> Option<Handle> {
        self.occupied.get(index).copied()
    }

    /// 当前可直接保留的新节点槽数；不包含等待退役或移除的节点。
    pub const fn available_nodes(&self) -> usize {
        self.free.len()
    }

    pub(crate) fn available_retirements(&self) -> usize {
        self.retired.len() - self.retired_len - self.retired_reserved
    }

    /// 已发布覆盖的单调版本；仅成功发布时改变。
    pub const fn coverage_version(&self) -> u64 {
        self.coverage_version
    }

    /// 待发布投影的单调版本；准备、撤销、锁定家族失效或发布时改变。
    pub const fn prepared_version(&self) -> u64 {
        self.prepared_version
    }

    /// 返回句柄对应的区块；过期句柄返回 `None`。
    pub fn chunk(&self, handle: Handle) -> Option<Chunk> {
        self.node(handle).map(|node| node.chunk)
    }

    /// 借用已完成或可见的资源，所有权仍属于状态。
    pub fn resource(&self, handle: Handle) -> Option<&R> {
        match &self.node(handle)?.phase {
            ResourcePhase::Ready(resource) | ResourcePhase::Visible(resource) => Some(resource),
            ResourcePhase::Refreshing { current, .. }
            | ResourcePhase::Replacement { current, .. } => Some(current),
            ResourcePhase::Empty | ResourcePhase::Requested(_) => None,
        }
    }

    /// 借用待发布的编辑结果；旧资源仍由 [`Self::resource`] 和 [`Self::visible`] 返回。
    pub fn replacement(&self, handle: Handle) -> Option<&R> {
        match &self.node(handle)?.phase {
            ResourcePhase::Replacement { next, .. } => Some(next),
            _ => None,
        }
    }

    /// 取回尚未发布的候选所有权，供宿主复用其中未受编辑影响的部分。
    ///
    /// `Ready` 恢复为空；编辑候选只取走 `next`，旧 `current` 继续可见并标记待刷新。
    /// 返回资源不进入退役队列，也不被复制或销毁。宿主调用前须结束对此未发布资源的
    /// 接缝/GPU准备访问；取回后负责重用或释放，并可为同句柄发起新请求。
    /// 无候选时返回 `None`，不取消在途 ticket；锁定家族返回 [`StateError::PendingPatch`]，
    /// 过期句柄返回 [`StateError::StaleHandle`]，失败保持资源与覆盖不变。
    pub fn take_unpublished(&mut self, handle: Handle) -> Result<Option<R>, StateError> {
        let node = self.node_mut(handle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if !matches!(
            node.phase,
            ResourcePhase::Ready(_) | ResourcePhase::Replacement { .. }
        ) {
            return Ok(None);
        }
        let resource = match std::mem::replace(&mut node.phase, ResourcePhase::Empty) {
            ResourcePhase::Ready(resource) => resource,
            ResourcePhase::Replacement { current, next } => {
                node.phase = ResourcePhase::Visible(current);
                node.invalidated = true;
                next
            }
            _ => unreachable!("已验证未发布候选"),
        };
        self.wake(handle);
        Ok(Some(resource))
    }

    /// 借用全部待确认变化发布后的资源；编辑替换返回新资源。
    pub fn prepared_resource(&self, handle: Handle) -> Option<&R> {
        let node = self.node(handle)?;
        if matches!(node.patch, Some(Kind::Replace)) {
            self.replacement(handle)
        } else {
            self.resource(handle)
        }
    }

    /// 节点在全部待确认变化共同发布后的视图中是否为叶。
    pub fn is_prepared_visible(&self, handle: Handle) -> bool {
        self.view_topology(handle, true) == Some(Topology::Leaf)
    }

    pub(crate) fn view_topology(&self, handle: Handle, prepared: bool) -> Option<Topology> {
        let node = self.node(handle)?;
        if !prepared {
            return Some(node.topology);
        }
        let Some(token) = node.pending else {
            return Some(node.topology);
        };
        let owner = self.node(token.handle)?;
        let Some(kind) = owner.patch else {
            return Some(node.topology);
        };
        Some(if token.handle == handle {
            match kind {
                Kind::Activate | Kind::Merge | Kind::Replace => Topology::Leaf,
                Kind::Remove => Topology::Detached,
                Kind::Split => Topology::Branch,
            }
        } else {
            match kind {
                Kind::Split => Topology::Leaf,
                Kind::Merge => Topology::Detached,
                _ => node.topology,
            }
        })
    }

    /// 全部待确认变化共同发布后的候选叶资源；完整遍历为 `O(len)`。
    ///
    /// 这是准备投影，不代表源/GPU已就绪或可提前撤旧覆盖；使用前须通过
    /// [`Self::validate_patch`] 检查全部待提交变化，最后由 [`Self::publish`] 交换当前视图。
    pub fn prepared_visible(&self) -> impl Iterator<Item = (Handle, Chunk, &R)> {
        self.occupied.iter().filter_map(|&handle| {
            let node = self.node(handle)?;
            if !self.is_prepared_visible(handle) {
                return None;
            }
            Some((handle, node.chunk, self.prepared_resource(handle)?))
        })
    }

    /// 该节点当前是否属于可见覆盖 cut。
    pub fn is_visible(&self, handle: Handle) -> bool {
        self.node(handle)
            .is_some_and(|node| node.topology == Topology::Leaf)
    }

    /// 尚未交回宿主销毁的资源数，不包含已预留的提交容量。
    pub const fn retired_len(&self) -> usize {
        self.retired_len
    }

    /// 尚未完成或取消的构建请求数；常数时间读取。
    pub const fn pending_requests(&self) -> usize {
        self.pending_requests
    }

    /// 已准备但尚未发布或撤销的覆盖变化数；常数时间读取。
    pub const fn pending_patches(&self) -> usize {
        self.pending_patches
    }

    /// 借用当前可见资源，依次扫描已占用节点。
    ///
    /// 完整遍历成本为 `O(len)`；增量消费者优先使用 [`Self::patch_resources`]。
    pub fn visible(&self) -> impl Iterator<Item = (Handle, Chunk, &R)> {
        self.occupied.iter().filter_map(|&handle| {
            let node = self.node(handle)?;
            let resource = match &node.phase {
                ResourcePhase::Visible(resource) => resource,
                ResourcePhase::Refreshing { current, .. }
                | ResourcePhase::Replacement { current, .. } => current,
                _ => return None,
            };
            Some((handle, node.chunk, resource))
        })
    }

    pub(crate) fn node(&self, handle: Handle) -> Option<&Node<R>> {
        let slot = self.slots.get(handle.index)?;
        (slot.generation == handle.generation)
            .then_some(slot.node.as_ref())
            .flatten()
    }

    pub(crate) fn visit_handle(&self, index: usize) -> Option<Handle> {
        let slot = self.slots.get(index)?;
        slot.node.as_ref()?;
        Some(Handle {
            index,
            generation: slot.generation,
        })
    }

    fn node_mut(&mut self, handle: Handle) -> Result<&mut Node<R>, StateError> {
        let slot = self
            .slots
            .get_mut(handle.index)
            .ok_or(StateError::StaleHandle)?;
        if slot.generation != handle.generation {
            return Err(StateError::StaleHandle);
        }
        slot.node.as_mut().ok_or(StateError::StaleHandle)
    }

    fn next_generation(&mut self) -> Result<u64, StateError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(StateError::GenerationExhausted)?;
        Ok(self.generation)
    }

    fn wake(&mut self, handle: Handle) {
        let parent = self.node(handle).and_then(|node| node.parent);
        for current in [Some(handle), parent].into_iter().flatten() {
            self.wake_one(current);
        }
    }

    fn wake_one(&mut self, handle: Handle) {
        if let Some(node) = self.node(handle) {
            let bucket = self.target.map_or(0, |target| {
                crate::queue::distance_bucket(self.config.distance2(node.chunk, target))
            });
            self.schedule.wake(handle, bucket);
        }
    }

    fn reserve(&mut self, chunk: Chunk, parent: Option<Handle>) -> Result<Handle, StateError> {
        if let Some(handle) = self.find(chunk) {
            return if self.node(handle).is_some_and(|node| node.parent == parent) {
                Ok(handle)
            } else {
                Err(StateError::Hierarchy)
            };
        }
        let index = *self.free.last().ok_or(StateError::Capacity)?;
        let generation = self.next_generation()?;
        self.free.pop();
        let handle = Handle { index, generation };
        self.slots[index] = Slot {
            generation,
            occupied_index: self.occupied.len(),
            node: Some(Node {
                chunk,
                parent,
                children: [None; 8],
                topology: Topology::Detached,
                phase: ResourcePhase::Empty,
                pending: None,
                invalidated: false,
                patch: None,
            }),
        };
        self.occupied.push(handle);
        self.index.insert(chunk, handle);
        self.wake(handle);
        Ok(handle)
    }

    pub(crate) fn reserve_root(&mut self, chunk: Chunk) -> Result<Handle, StateError> {
        if chunk.lod != self.config.max_lod() {
            return Err(StateError::Hierarchy);
        }
        self.reserve(chunk, None)
    }

    pub(crate) fn reserve_child(
        &mut self,
        parent: Handle,
        octant: usize,
    ) -> Result<Handle, StateError> {
        let node = self.node(parent).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if node.chunk.lod == 0 || octant >= 8 {
            return Err(StateError::Hierarchy);
        }
        if let Some(handle) = node.children[octant] {
            return Ok(handle);
        }
        let mut coord = node.chunk.coord;
        for (axis, value) in coord.iter_mut().enumerate() {
            *value = value
                .checked_mul(2)
                .and_then(|value| value.checked_add(((octant >> axis) & 1) as i64))
                .ok_or(StateError::Hierarchy)?;
        }
        let chunk = Chunk {
            coord,
            lod: node.chunk.lod - 1,
        };
        let handle = self.reserve(chunk, Some(parent))?;
        self.node_mut(parent)?.children[octant] = Some(handle);
        self.wake(parent);
        Ok(handle)
    }

    /// 发起一个空资源节点的构建；重复请求返回原有代号。
    pub fn request(&mut self, handle: Handle) -> Result<Build, StateError> {
        self.request_inner(handle, true)
    }

    pub(crate) fn request_planned(&mut self, handle: Handle) -> Result<Build, StateError> {
        self.request_inner(handle, false)
    }

    fn request_inner(&mut self, handle: Handle, wake_self: bool) -> Result<Build, StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if let ResourcePhase::Requested(ticket) = node.phase {
            return Ok(Build {
                ticket,
                chunk: node.chunk,
            });
        }
        if !matches!(node.phase, ResourcePhase::Empty) {
            return Err(StateError::Transition);
        }
        let chunk = node.chunk;
        let ticket = Ticket {
            handle,
            generation: self.next_generation()?,
        };
        self.node_mut(handle)?.phase = ResourcePhase::Requested(ticket);
        self.node_mut(handle)?.invalidated = false;
        self.pending_requests += 1;
        self.wake_requested(handle, wake_self);
        Ok(Build { ticket, chunk })
    }

    /// 为可见叶发起编辑刷新，保留旧资源至完整替换发布。
    pub fn request_refresh(&mut self, handle: Handle) -> Result<Build, StateError> {
        self.request_refresh_inner(handle, true)
    }

    pub(crate) fn request_refresh_planned(&mut self, handle: Handle) -> Result<Build, StateError> {
        self.request_refresh_inner(handle, false)
    }

    fn request_refresh_inner(
        &mut self,
        handle: Handle,
        wake_self: bool,
    ) -> Result<Build, StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if let ResourcePhase::Refreshing { ticket, .. } = node.phase {
            return Ok(Build {
                ticket,
                chunk: node.chunk,
            });
        }
        if !matches!(node.phase, ResourcePhase::Visible(_)) {
            return Err(StateError::Transition);
        }
        let chunk = node.chunk;
        let ticket = Ticket {
            handle,
            generation: self.next_generation()?,
        };
        let node = self.node_mut(handle)?;
        let ResourcePhase::Visible(current) =
            std::mem::replace(&mut node.phase, ResourcePhase::Empty)
        else {
            unreachable!("已验证可见资源")
        };
        node.phase = ResourcePhase::Refreshing { current, ticket };
        node.invalidated = false;
        self.pending_requests += 1;
        self.wake_requested(handle, wake_self);
        Ok(Build { ticket, chunk })
    }

    fn wake_requested(&mut self, handle: Handle, wake_self: bool) {
        if wake_self {
            self.wake(handle);
        } else if let Some(parent) = self.node(handle).and_then(|node| node.parent) {
            // 规划器已协调本节点；请求完成/失败或新目标会重新唤醒它，只需继续补齐兄弟。
            self.wake_one(parent);
        }
    }

    /// 接受匹配请求的完成结果；过期结果连同错误原样返还。
    ///
    /// 完成表示宿主已得到完整有效的构建资源，构建过程不再异步访问它；
    /// 不改变当前可见覆盖。数据未知、未加载、失败或容量压力不能用空占位资源完成，
    /// 只有确知真实为空的输入才允许完成空几何；这些源语义由宿主建立，State不解释R。
    /// 未发布资源允许立即进入释放队列；接缝/GPU准备中的访问须在发布或撤销前结束。
    pub fn complete(&mut self, ticket: Ticket, resource: R) -> Result<(), Rejected<R>> {
        if !self.node(ticket.handle).is_some_and(
            |node| matches!(node.phase, ResourcePhase::Requested(current) | ResourcePhase::Refreshing { ticket: current, .. } if current == ticket),
        ) {
            return Err(Rejected {
                error: StateError::StaleTicket,
                resource,
            });
        }
        let node = self.slots[ticket.handle.index]
            .node
            .as_mut()
            .expect("已验证节点");
        node.phase = match std::mem::replace(&mut node.phase, ResourcePhase::Empty) {
            ResourcePhase::Requested(_) => ResourcePhase::Ready(resource),
            ResourcePhase::Refreshing { current, .. } => ResourcePhase::Replacement {
                current,
                next: resource,
            },
            _ => unreachable!("已验证请求"),
        };
        self.pending_requests -= 1;
        self.wake(ticket.handle);
        Ok(())
    }

    /// 失败后保留旧可见资源；没有旧资源时恢复为空，后续规划可重试。
    pub fn fail(&mut self, ticket: Ticket) -> Result<(), StateError> {
        let node = self
            .node_mut(ticket.handle)
            .map_err(|_| StateError::StaleTicket)?;
        if !matches!(node.phase, ResourcePhase::Requested(current) | ResourcePhase::Refreshing { ticket: current, .. } if current == ticket)
        {
            return Err(StateError::StaleTicket);
        }
        node.phase = match std::mem::replace(&mut node.phase, ResourcePhase::Empty) {
            ResourcePhase::Requested(_) => ResourcePhase::Empty,
            ResourcePhase::Refreshing { current, .. } => {
                node.invalidated = true;
                ResourcePhase::Visible(current)
            }
            _ => unreachable!("已验证请求"),
        };
        self.pending_requests -= 1;
        self.wake(ticket.handle);
        Ok(())
    }

    /// 取消一个请求；之后到达的完成结果会返还其资源。
    pub fn cancel(&mut self, ticket: Ticket) -> Result<(), StateError> {
        self.fail(ticket)
    }

    /// 准备启用根节点，或补齐现有分支中已完成的一个缺口。
    pub fn prepare_activate(&mut self, handle: Handle) -> Result<Patch, StateError> {
        self.prepare(handle, Kind::Activate)
    }

    /// 为已完成的编辑结果预留退役容量，发布前保持旧可见资源。
    pub fn prepare_replace(&mut self, handle: Handle) -> Result<Patch, StateError> {
        self.prepare(handle, Kind::Replace)
    }

    /// 准备移除一个可见叶；设置目标后只能移除需求根域之外的叶。
    ///
    /// 退出需求域允许逐叶回收，不必为释放远处资源重新构建父网格。
    pub fn prepare_remove(&mut self, handle: Handle) -> Result<Patch, StateError> {
        self.prepare(handle, Kind::Remove)
    }

    /// 八子节点都完成且整个家族未编辑失效时，准备替换父叶；发布前仍显示父叶。
    pub fn prepare_split(&mut self, handle: Handle) -> Result<Patch, StateError> {
        self.prepare(handle, Kind::Split)
    }

    /// 父资源完成且八子节点均为未刷新的可见叶、整个家族未失效时，准备反向替换。
    /// 发布前八子节点继续组成当前覆盖。
    pub fn prepare_merge(&mut self, handle: Handle) -> Result<Patch, StateError> {
        self.prepare(handle, Kind::Merge)
    }

    fn prepare(&mut self, handle: Handle, kind: Kind) -> Result<Patch, StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        let chunk = node.chunk;
        let family = node.children;
        let valid = match kind {
            Kind::Replace => {
                node.topology == Topology::Leaf
                    && matches!(node.phase, ResourcePhase::Replacement { .. })
                    && !node.invalidated
            }
            Kind::Activate => {
                self.can_activate(handle)
                    && node.topology == Topology::Detached
                    && matches!(node.phase, ResourcePhase::Ready(_))
                    && !node.invalidated
            }
            Kind::Remove => {
                ((node.parent.is_none() && self.target.is_none())
                    || self.outside_target_root(node.chunk))
                    && node.topology == Topology::Leaf
                    && matches!(node.phase, ResourcePhase::Visible(_))
            }
            Kind::Split => {
                node.topology == Topology::Leaf
                    && matches!(node.phase, ResourcePhase::Visible(_))
                    && !node.invalidated
            }
            Kind::Merge => {
                node.topology == Topology::Branch
                    && matches!(node.phase, ResourcePhase::Ready(_))
                    && !node.invalidated
            }
        };
        if !valid {
            return Err(StateError::Transition);
        }
        let retired = kind.retired();
        let mut child_chunks = [chunk; 8];
        if matches!(kind, Kind::Split | Kind::Merge) {
            for (index, child) in family.into_iter().enumerate() {
                let child = child.ok_or(StateError::Transition)?;
                let node = self.node(child).ok_or(StateError::StaleHandle)?;
                if node.pending.is_some() {
                    return Err(StateError::PendingPatch);
                }
                let valid = match kind {
                    Kind::Split => {
                        node.topology == Topology::Detached
                            && matches!(node.phase, ResourcePhase::Ready(_))
                    }
                    Kind::Merge => {
                        node.topology == Topology::Leaf
                            && matches!(node.phase, ResourcePhase::Visible(_))
                    }
                    _ => false,
                };
                if !valid || node.parent != Some(handle) {
                    return Err(StateError::Transition);
                }
                if node.invalidated {
                    return Err(StateError::Transition);
                }
                child_chunks[index] = node.chunk;
            }
        }
        if self.retired_len + self.retired_reserved + retired > self.retired.len() {
            return Err(StateError::RetirementFull);
        }
        let token = PatchToken {
            handle,
            generation: self.next_generation()?,
        };
        let children = if matches!(kind, Kind::Split | Kind::Merge) {
            family
        } else {
            [None; 8]
        };
        let node = self.node_mut(handle)?;
        node.pending = Some(token);
        node.patch = Some(kind);
        for child in children.into_iter().flatten() {
            self.node_mut(child)?.pending = Some(token);
        }
        self.retired_reserved += retired;
        self.pending_patches += 1;
        self.prepared_version = token.generation;
        let change = match kind {
            Kind::Replace => Change::Replace(chunk),
            Kind::Activate => Change::Activate(chunk),
            Kind::Remove => Change::Remove(chunk),
            Kind::Split => Change::Split {
                parent: chunk,
                children: child_chunks,
            },
            Kind::Merge => Change::Merge {
                parent: chunk,
                children: child_chunks,
            },
        };
        Ok(Patch { token, change })
    }

    fn can_activate(&self, handle: Handle) -> bool {
        let Some(node) = self.node(handle) else {
            return false;
        };
        match node.parent {
            None => true,
            Some(parent) => self.node(parent).is_some_and(|parent| {
                parent.topology == Topology::Branch
                    && parent.pending.is_none()
                    && parent.children.contains(&Some(handle))
            }),
        }
    }

    fn outside_target_root(&self, chunk: Chunk) -> bool {
        let Some(target) = self.target else {
            return false;
        };
        let shift = self.config.max_lod() - chunk.lod;
        let divisor = 1_i64 << shift;
        let root = Chunk {
            coord: chunk.coord.map(|value| value.div_euclid(divisor)),
            lod: self.config.max_lod(),
        };
        !self.config.contains_root(root, target)
    }

    fn pending(&self, token: PatchToken) -> Result<Pending, StateError> {
        let node = self.node(token.handle).ok_or(StateError::StalePatch)?;
        let kind = node.patch.ok_or(StateError::StalePatch)?;
        if node.pending != Some(token) {
            return Err(StateError::StalePatch);
        }
        let children = if matches!(kind, Kind::Split | Kind::Merge) {
            // 家族锁阻止预留或释放子槽，读取唯一拓扑即可恢复本次事务。
            for child in node.children {
                let child = child.ok_or(StateError::StalePatch)?;
                if !self
                    .node(child)
                    .is_some_and(|node| node.pending == Some(token))
                {
                    return Err(StateError::StalePatch);
                }
            }
            node.children
        } else {
            [None; 8]
        };
        Ok(Pending {
            token,
            kind,
            children,
        })
    }

    /// 撤销待确认变化，释放节点锁与预留容量，资源保持原状。
    ///
    /// 撤销前宿主须结束对未发布资源的接缝准备访问；随后它们可能被回收。
    pub fn reject_patch(&mut self, token: PatchToken) -> Result<(), StateError> {
        let pending = self.pending(token)?;
        self.prepared_version = self.next_generation()?;
        self.unlock(pending);
        Ok(())
    }

    fn unlock(&mut self, pending: Pending) {
        let root = self.slots[pending.token.handle.index]
            .node
            .as_mut()
            .expect("已锁定节点");
        root.pending = None;
        root.patch = None;
        self.retired_reserved -= pending.kind.retired();
        self.pending_patches -= 1;
        self.wake(pending.token.handle);
        for child in pending.children.into_iter().flatten() {
            self.slots[child.index]
                .node
                .as_mut()
                .expect("已锁定子节点")
                .pending = None;
            self.wake(child);
        }
    }

    /// 宿主准备好最终覆盖、接缝和可用GPU资源后，在同一帧边界原子发布完整变化。
    ///
    /// `last_use` 是旧可见资源最后一次可能被 GPU 使用的提交值。宿主须将
    /// [`Patch::change`] 对应的渲染集合与接缝一同应用；本方法只维护覆盖拓扑，
    /// 不查询GPU。新资源须已完成所需准备，或由宿主提交依赖保证首次使用时可用；
    /// 构建/上传失败、未知源和压力等待都应继续显示旧覆盖，不得提前publish占位。
    /// 准备时已经预留全部释放槽，因此有效代号不会因队列满而部分发布。
    /// 家族锁不保护相邻家族；宿主须按最终邻接准备接缝，邻接变化后重新准备。
    /// 锁定家族任一节点编辑失效，或目标返回待移除区域时，返回
    /// [`StateError::Transition`]，不修改覆盖、退役队列或释放锁；
    /// 宿主应丢弃尚未启用的新绘制集合，并调用 [`Self::reject_patch`]。
    pub fn publish(&mut self, token: PatchToken, last_use: u64) -> Result<(), StateError> {
        self.publish_with(token, last_use, |_, _| {})
    }

    /// 验证替换后，将旧资源的可复用所有权转移给候选，再发布。
    ///
    /// 仅 [`Change::Replace`] 调用 `transfer(current, next)`；其他变化与 [`Self::publish`]
    /// 相同。所有可返回的错误及发布代号预留均先于回调，失败不会调用回调或修改资源。
    /// 回调返回后无额外失败边界；旧资源剩余部分仍按 `last_use` 退役。
    ///
    /// 回调只能进行不会失败的所有权重组。宿主须先结束被转移部分的全部旧消费者，
    /// 包括 GPU；`last_use` 仍约束未转移部分，不能替代此完成证明。回调不得 panic；
    /// 若它 panic，资源可能已部分转移，调用者必须销毁此 State，不能继续发布或回滚。
    /// 该资源语义与外部完成条件由实际宿主维护，不由覆盖拓扑校验或本接口证明。
    pub fn publish_with(
        &mut self,
        token: PatchToken,
        last_use: u64,
        transfer: impl FnOnce(&mut R, &mut R),
    ) -> Result<(), StateError> {
        let pending = self.validated_pending(token)?;
        let version = self.next_generation()?;
        if matches!(pending.kind, Kind::Replace) {
            let phase = &mut self.slots[token.handle.index]
                .node
                .as_mut()
                .expect("已验证替换节点")
                .phase;
            let ResourcePhase::Replacement { current, next } = phase else {
                unreachable!("已验证替换资源");
            };
            transfer(current, next);
        }
        self.publish_pending(pending, version, last_use);
        Ok(())
    }

    /// 提交 GPU 变化前检查编辑与需求域是否仍允许本次发布，不改变状态。
    ///
    /// 检查后至 [`Self::publish`] 之间，宿主应在同一所有者内避免再提交编辑或新目标。
    pub fn validate_patch(&self, token: PatchToken) -> Result<(), StateError> {
        self.validated_pending(token).map(|_| ())
    }

    /// 校验单项待发布事务并借用其可见资源增量，不枚举或复制完整覆盖。
    ///
    /// 返回最多九项，先删除旧叶，再插入新叶；编辑替换只返回同身份的新资源。
    /// 宿主可用它维护仅含键/句柄的视图；资源所有权始终属于本状态。
    /// 借用结束后若发生编辑、目标或邻接变化，提交前仍须重新校验并准备最终 cut。
    /// 过期或失效事务与 [`Self::validate_patch`] 返回相同错误，失败不修改状态。
    pub fn patch_resources(
        &self,
        token: PatchToken,
    ) -> Result<crate::PatchResources<'_, R>, StateError> {
        let pending = self.validated_pending(token)?;
        let mut entries = [(token.handle, false); 9];
        let len = match pending.kind {
            Kind::Activate | Kind::Replace => {
                entries[0] = (token.handle, true);
                1
            }
            Kind::Remove => 1,
            Kind::Split => {
                for (entry, child) in entries[1..].iter_mut().zip(pending.children) {
                    *entry = (child.expect("已验证八子家族"), true);
                }
                9
            }
            Kind::Merge => {
                for (entry, child) in entries[..8].iter_mut().zip(pending.children) {
                    *entry = (child.expect("已验证八子家族"), false);
                }
                entries[8] = (token.handle, true);
                9
            }
        };
        Ok(crate::PatchResources {
            state: self,
            entries,
            next: 0,
            len,
        })
    }

    fn validated_pending(&self, token: PatchToken) -> Result<Pending, StateError> {
        let pending = self.pending(token)?;
        let fresh = match pending.kind {
            Kind::Activate | Kind::Replace => {
                !self.node(token.handle).expect("已锁定节点").invalidated
            }
            Kind::Split | Kind::Merge => {
                !self.node(token.handle).expect("已锁定节点").invalidated
                    && pending
                        .children
                        .into_iter()
                        .flatten()
                        .all(|child| !self.node(child).expect("已锁定节点").invalidated)
            }
            Kind::Remove => true,
        };
        if !fresh {
            return Err(StateError::Transition);
        }
        if matches!(pending.kind, Kind::Remove) {
            let node = self.node(token.handle).ok_or(StateError::StalePatch)?;
            if self.target.is_some() && !self.outside_target_root(node.chunk) {
                return Err(StateError::Transition);
            }
        }
        Ok(pending)
    }

    fn publish_pending(&mut self, pending: Pending, version: u64, last_use: u64) {
        let token = pending.token;
        match pending.kind {
            Kind::Replace => {
                let node = self.slots[token.handle.index]
                    .node
                    .as_mut()
                    .expect("已锁定节点");
                let ResourcePhase::Replacement { current, next } =
                    std::mem::replace(&mut node.phase, ResourcePhase::Empty)
                else {
                    unreachable!("已验证替换资源")
                };
                node.phase = ResourcePhase::Visible(next);
                self.push_retired(current, last_use);
            }
            Kind::Activate => {
                self.make_visible(token.handle);
            }
            Kind::Remove => {
                self.retire_visible(token.handle, last_use);
                self.slots[token.handle.index]
                    .node
                    .as_mut()
                    .expect("已锁定节点")
                    .topology = Topology::Detached;
            }
            Kind::Split => {
                self.retire_visible(token.handle, last_use);
                self.slots[token.handle.index]
                    .node
                    .as_mut()
                    .expect("已锁定节点")
                    .topology = Topology::Branch;
                for child in pending.children.into_iter().flatten() {
                    self.make_visible(child);
                }
            }
            Kind::Merge => {
                for child in pending.children.into_iter().flatten() {
                    self.retire_visible(child, last_use);
                    self.slots[child.index]
                        .node
                        .as_mut()
                        .expect("已锁定子节点")
                        .topology = Topology::Detached;
                }
                self.make_visible(token.handle);
            }
        }
        self.coverage_version = version;
        self.prepared_version = version;
        self.unlock(pending);
    }

    fn make_visible(&mut self, handle: Handle) {
        let node = self.slots[handle.index].node.as_mut().expect("已锁定节点");
        let ResourcePhase::Ready(resource) =
            std::mem::replace(&mut node.phase, ResourcePhase::Empty)
        else {
            unreachable!("发布前已验证资源状态")
        };
        node.phase = ResourcePhase::Visible(resource);
        node.topology = Topology::Leaf;
    }

    fn retire_visible(&mut self, handle: Handle, after: u64) {
        let node = self.slots[handle.index].node.as_mut().expect("已锁定节点");
        let ResourcePhase::Visible(resource) =
            std::mem::replace(&mut node.phase, ResourcePhase::Empty)
        else {
            unreachable!("发布前已验证资源状态")
        };
        self.push_retired(resource, after);
    }

    fn push_retired(&mut self, resource: R, after: u64) {
        let tail = (self.retired_head + self.retired_len) % self.retired.len();
        debug_assert!(self.retired[tail].is_none());
        self.retired[tail] = Some(Retired { resource, after });
        self.retired_len += 1;
    }

    pub(crate) fn retire_ready(&mut self, handle: Handle) -> Result<(), StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if !matches!(node.phase, ResourcePhase::Ready(_)) {
            return Err(StateError::Transition);
        }
        if self.retired_len + self.retired_reserved == self.retired.len() {
            return Err(StateError::RetirementFull);
        }
        let node = self.node_mut(handle)?;
        let ResourcePhase::Ready(resource) =
            std::mem::replace(&mut node.phase, ResourcePhase::Empty)
        else {
            unreachable!("已验证资源状态")
        };
        self.push_retired(resource, 0);
        self.wake(handle);
        Ok(())
    }

    pub(crate) fn discard_replacement(&mut self, handle: Handle) -> Result<(), StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if !matches!(node.phase, ResourcePhase::Replacement { .. }) {
            return Err(StateError::Transition);
        }
        if self.retired_len + self.retired_reserved == self.retired.len() {
            return Err(StateError::RetirementFull);
        }
        let node = self.node_mut(handle)?;
        let ResourcePhase::Replacement { current, next } =
            std::mem::replace(&mut node.phase, ResourcePhase::Empty)
        else {
            unreachable!("已验证替换资源")
        };
        node.phase = ResourcePhase::Visible(current);
        self.push_retired(next, 0);
        self.wake(handle);
        Ok(())
    }

    pub(crate) fn remove_empty(&mut self, handle: Handle) -> Result<(), StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if node.topology != Topology::Detached
            || !matches!(node.phase, ResourcePhase::Empty)
            || node.children.iter().any(Option::is_some)
        {
            return Err(StateError::Transition);
        }
        let parent = node.parent;
        let chunk = node.chunk;
        if let Some(parent) = parent {
            let node = self.node_mut(parent)?;
            if node.pending.is_some() {
                return Err(StateError::PendingPatch);
            }
            let child = node
                .children
                .iter_mut()
                .find(|child| **child == Some(handle))
                .ok_or(StateError::Hierarchy)?;
            *child = None;
        }
        self.index.remove(&chunk);
        let position = self.slots[handle.index].occupied_index;
        self.occupied.swap_remove(position);
        if let Some(&moved) = self.occupied.get(position) {
            self.slots[moved.index].occupied_index = position;
            self.wake(moved);
        }
        self.schedule.removed_occupied(self.occupied.len());
        self.schedule.discard(handle);
        self.slots[handle.index].node = None;
        self.free.push(handle.index);
        if let Some(parent) = parent {
            self.wake(parent);
        }
        Ok(())
    }

    pub(crate) fn detach_empty(&mut self, handle: Handle) -> Result<(), StateError> {
        let node = self.node(handle).ok_or(StateError::StaleHandle)?;
        if node.pending.is_some() {
            return Err(StateError::PendingPatch);
        }
        if node.topology == Topology::Leaf
            || !matches!(node.phase, ResourcePhase::Empty)
            || node.children.iter().any(Option::is_some)
            || !self.outside_target_root(node.chunk)
        {
            return Err(StateError::Transition);
        }
        self.node_mut(handle)?.topology = Topology::Detached;
        self.wake(handle);
        Ok(())
    }

    /// 按释放队列顺序交回至多一个已完成使用的资源。
    ///
    /// 单次只检查队首。`completed` 及之前的提交须已全部结束；宿主负责销毁
    /// 返回的资源。队首未就绪时返回 `None`，不会扫描后续项目。
    pub fn take_retired(&mut self, completed: u64) -> Option<R> {
        let retired = self.retired[self.retired_head].as_ref()?;
        if retired.after > completed {
            return None;
        }
        let retired = self.retired[self.retired_head].take().expect("已检查队首");
        self.retired_head = (self.retired_head + 1) % self.retired.len();
        self.retired_len -= 1;
        Some(retired.resource)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(capacity: usize) -> State<u32> {
        State::new(Config::new(2, 4, capacity).unwrap())
    }

    fn ready(state: &mut State<u32>, handle: Handle, resource: u32) {
        let build = state.request(handle).unwrap();
        state.complete(build.ticket, resource).unwrap();
    }

    #[test]
    fn stale_completion_returns_ownership() {
        let mut state = state(16);
        let handle = state
            .reserve_root(Chunk {
                coord: [0; 3],
                lod: 1,
            })
            .unwrap();
        let old = state.request(handle).unwrap();
        state.cancel(old.ticket).unwrap();
        let current = state.request(handle).unwrap();
        let rejected = state.complete(old.ticket, 7).unwrap_err();
        assert_eq!(rejected.resource, 7);
        state.complete(current.ticket, 8).unwrap();
        assert_eq!(state.resource(handle), Some(&8));
    }

    #[test]
    fn split_publishes_the_whole_cover() {
        let mut state = state(16);
        let parent = state
            .reserve_root(Chunk {
                coord: [0; 3],
                lod: 1,
            })
            .unwrap();
        ready(&mut state, parent, 10);
        let patch = state.prepare_activate(parent).unwrap();
        state.publish(patch.token, 0).unwrap();
        let children = std::array::from_fn::<_, 8, _>(|octant| {
            let child = state.reserve_child(parent, octant).unwrap();
            ready(&mut state, child, octant as u32);
            child
        });
        let patch = state.prepare_split(parent).unwrap();
        assert!(state.is_visible(parent));
        assert!(children.iter().all(|child| !state.is_visible(*child)));
        assert_eq!(state.prepare_split(parent), Err(StateError::PendingPatch));
        state.publish(patch.token, 3).unwrap();
        assert!(!state.is_visible(parent));
        assert!(children.iter().all(|child| state.is_visible(*child)));
        assert_eq!(state.take_retired(2), None);
        assert_eq!(state.take_retired(3), Some(10));
    }

    #[test]
    fn rejected_patch_keeps_resources_and_unlocks_nodes() {
        let mut state = state(16);
        let handle = state
            .reserve_root(Chunk {
                coord: [0; 3],
                lod: 1,
            })
            .unwrap();
        ready(&mut state, handle, 5);
        let patch = state.prepare_activate(handle).unwrap();
        state.reject_patch(patch.token).unwrap();
        assert!(!state.is_visible(handle));
        assert_eq!(state.resource(handle), Some(&5));
        assert!(state.prepare_activate(handle).is_ok());
        assert_eq!(state.publish(patch.token, 0), Err(StateError::StalePatch));
    }

    #[test]
    fn reused_slot_does_not_accept_an_old_handle() {
        let mut state = state(16);
        let chunk = Chunk {
            coord: [0; 3],
            lod: 1,
        };
        let old = state.reserve_root(chunk).unwrap();
        state.remove_empty(old).unwrap();
        let current = state.reserve_root(chunk).unwrap();
        assert_ne!(old, current);
        assert_eq!(state.chunk(old), None);
        assert_eq!(state.chunk(current), Some(chunk));
    }

    #[test]
    fn retirement_backpressure_keeps_the_visible_cover() {
        let mut state = state(1);
        let root = state
            .reserve_root(Chunk {
                coord: [0; 3],
                lod: 1,
            })
            .unwrap();
        ready(&mut state, root, 1);
        let patch = state.prepare_activate(root).unwrap();
        state.publish(patch.token, 0).unwrap();
        let patch = state.prepare_remove(root).unwrap();
        state.publish(patch.token, 0).unwrap();
        ready(&mut state, root, 2);
        let patch = state.prepare_activate(root).unwrap();
        state.publish(patch.token, 0).unwrap();
        assert_eq!(state.prepare_remove(root), Err(StateError::RetirementFull));
        assert!(state.is_visible(root));
    }

    #[test]
    fn obsolete_branch_is_reclaimed_without_rebuilding_parent() {
        let mut state = state(16);
        let root = state
            .reserve_root(Chunk {
                coord: [0; 3],
                lod: 1,
            })
            .unwrap();
        ready(&mut state, root, 1);
        let patch = state.prepare_activate(root).unwrap();
        state.publish(patch.token, 0).unwrap();
        let children = std::array::from_fn::<_, 8, _>(|octant| {
            let child = state.reserve_child(root, octant).unwrap();
            ready(&mut state, child, octant as u32);
            child
        });
        let patch = state.prepare_split(root).unwrap();
        state.publish(patch.token, 0).unwrap();
        state.target = Some(Target::new([1000; 3]).unwrap());
        for child in children {
            let patch = state.prepare_remove(child).unwrap();
            state.publish(patch.token, 0).unwrap();
            state.remove_empty(child).unwrap();
        }
        state.detach_empty(root).unwrap();
        state.remove_empty(root).unwrap();
        assert!(state.is_empty());
    }

    #[test]
    fn replacement_keeps_the_old_resource_until_publish_and_fence() {
        let mut state = state(16);
        let handle = state.reserve_root(Chunk::new([0; 3], 1)).unwrap();
        ready(&mut state, handle, 1);
        let patch = state.prepare_activate(handle).unwrap();
        state.publish(patch.token, 0).unwrap();
        let build = state.request_refresh(handle).unwrap();
        assert_eq!(state.resource(handle), Some(&1));
        state.complete(build.ticket, 2).unwrap();
        let patch = state.prepare_replace(handle).unwrap();
        assert_eq!(state.resource(handle), Some(&1));
        assert_eq!(state.prepared_resource(handle), Some(&2));
        state.publish(patch.token, 10).unwrap();
        assert_eq!(state.resource(handle), Some(&2));
        assert_eq!(state.take_retired(9), None);
        assert_eq!(state.take_retired(10), Some(1));
    }

    #[test]
    fn invalidation_rejects_an_already_prepared_replacement() {
        let mut state = state(16);
        let handle = state.reserve_root(Chunk::new([0; 3], 1)).unwrap();
        ready(&mut state, handle, 1);
        let patch = state.prepare_activate(handle).unwrap();
        state.publish(patch.token, 0).unwrap();
        let build = state.request_refresh(handle).unwrap();
        state.complete(build.ticket, 2).unwrap();
        let patch = state.prepare_replace(handle).unwrap();
        state.invalidate(handle).unwrap();
        assert_eq!(
            state.validate_patch(patch.token),
            Err(StateError::Transition)
        );
        assert_eq!(state.publish(patch.token, 0), Err(StateError::Transition));
        assert_eq!(state.resource(handle), Some(&1));
        state.reject_patch(patch.token).unwrap();
        state.discard_replacement(handle).unwrap();
        assert_eq!(state.take_retired(0), Some(2));
    }
}
