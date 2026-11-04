use crate::{
    Build, Chunk, Config, CursorStep, Handle, LevelDiffCursor, Patch, RootChange, RootCursor,
    RootDiffCursor, State, StateError, Target,
    bounds_diff::BoundsDiff,
    queue::{DistanceQueue, distance_bucket},
    state::{ResourcePhase, Topology},
};
use std::collections::VecDeque;

const WINDOW_DIFFS: usize = 8;

/// 一次规划的独立上限；零额度不会被提升为默认值。
///
/// `visits` 计游标推进、空槽检查、距离重排与节点协调，包含未产出请求的工作。
/// 构建及覆盖发布另设额度；完成消息和 [`State::take_retired`] 每次只处理一个结果，
/// 宿主也须限制每帧调用次数。资源本身的构建、上传和销毁成本不由本额度估计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// 最多执行的有界管理步骤。
    pub visits: usize,
    /// 最多新发起的异步构建请求。
    pub requests: usize,
    /// 最多准备的完整覆盖替换；父子替换整体计一次。
    pub switches: usize,
}

/// 单次规划实际消耗，不扫描状态计算剩余差分数量。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// 已执行的管理步骤，包含无输出的检查。
    pub visits: usize,
    /// 本批新请求数。
    pub requests: usize,
    /// 本批已取消的在途请求数；宿主可据此停止外部建网。
    pub cancellations: usize,
    /// 本批待确认的覆盖替换数。
    pub switches: usize,
    /// 遇到容量或本批输出额度限制的步骤数；已经暂停的等待项不重复计步。
    pub blocked: usize,
    /// 是否存在节点或退役容量阻塞；不包含单纯请求/发布输出额度不足。
    ///
    /// 宿主可据此在帧外按实际占用扩容；本方法不推算整个视距体积。
    pub capacity_pressure: bool,
    /// 是否还有游标、协调工作、在途请求或待确认替换。
    ///
    /// 不包含等待 GPU 完成的退役资源；使用 [`State::retired_len`] 单独查询。
    pub pending: bool,
}

/// 规划的可恢复错误。
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// 上一批元数据尚未消费；状态没有变化。
    #[error("上一批计划尚未清空")]
    NotEmpty,
    /// 状态无法继续转换；已经生成的本批请求仍须消费。
    #[error("区块状态转换失败：{0}")]
    State(#[from] StateError),
}

/// 可复用的有限输出缓冲，只持有请求与句柄，不持有资源或覆盖镜像。
///
/// 请求产生时状态已经记录在途 ticket；替换产生时状态已经锁定局部家族，
/// 但仍显示原覆盖。宿主必须逐项完成、失败或取消请求，并发布或拒绝替换。
/// [`Self::clear`] 只回收元数据容量，不代替这些状态转换。
pub struct Plan {
    requests: Vec<Build>,
    cancellations: Vec<crate::Ticket>,
    switches: Vec<Patch>,
}

impl Plan {
    /// 帧外为新请求和取消 ticket 各预留 `requests` 项，为替换预留 `switches` 项。
    ///
    /// 后续规划不扩容；输出满时暂停相关转换。
    pub fn with_capacity(requests: usize, switches: usize) -> Self {
        Self {
            requests: Vec::with_capacity(requests),
            cancellations: Vec::with_capacity(requests),
            switches: Vec::with_capacity(switches),
        }
    }

    /// 按发现时的距离桶顺序借用本批请求。
    pub fn requests(&self) -> &[Build] {
        &self.requests
    }

    /// 已在状态中取消的 ticket；宿主可取消尚未结束的外部任务。
    ///
    /// 若外部任务无法取消，完成结果会由 [`State::complete`] 原样返还。
    pub fn cancellations(&self) -> &[crate::Ticket] {
        &self.cancellations
    }

    /// 借用本批完整替换；应用接缝后调用 [`State::publish`]，推迟则保留 token。
    pub fn switches(&self) -> &[Patch] {
        &self.switches
    }

    /// 清空已消费的 POD 元数据，保留全部缓冲容量。
    pub fn clear(&mut self) {
        self.requests.clear();
        self.cancellations.clear();
        self.switches.clear();
    }

    /// 是否没有尚待消费的输出。
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.cancellations.is_empty() && self.switches.is_empty()
    }

    /// 输出对象和保留缓冲的字节数，不包含状态或宿主资源。
    pub fn reserved_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.requests.capacity() * std::mem::size_of::<Build>()
            + self.cancellations.capacity() * std::mem::size_of::<crate::Ticket>()
            + self.switches.capacity() * std::mem::size_of::<Patch>()
    }
}

// 只记录未完成的协调工作；资源、可见性与请求代号始终由 State 的节点维护。
pub(crate) struct Schedule {
    queue: DistanceQueue,
    retry: DistanceQueue,
    roots: Option<RootCursor>,
    roots_again: bool,
    root_pending: Option<Chunk>,
    capacity_checked: bool,
    config_before: Option<Config>,
    bounds_diffs: VecDeque<BoundsDiff>,
    root_diff: Option<RootDiffCursor>,
    diffs: [Option<LevelDiffCursor>; 31],
    diff_mask: u32,
    diff_turn: u8,
    diff_root_turn: bool,
    scan_index: usize,
    scan_remaining: usize,
    turn: u8,
    distances_current: bool,
}

impl Schedule {
    pub(crate) fn new(config: Config) -> Self {
        Self {
            queue: DistanceQueue::new(config.capacity()),
            retry: DistanceQueue::new(config.capacity()),
            roots: None,
            roots_again: false,
            root_pending: None,
            capacity_checked: false,
            config_before: None,
            bounds_diffs: VecDeque::with_capacity(WINDOW_DIFFS),
            root_diff: None,
            diffs: std::array::from_fn(|_| None),
            diff_mask: 0,
            diff_turn: 0,
            diff_root_turn: true,
            scan_index: 0,
            scan_remaining: 0,
            turn: 0,
            distances_current: true,
        }
    }

    pub(crate) fn wake(&mut self, handle: Handle, bucket: u8) {
        self.retry.discard(handle);
        self.queue.insert(handle, bucket);
    }

    pub(crate) fn discard(&mut self, handle: Handle) {
        self.queue.discard(handle);
        self.retry.discard(handle);
    }

    pub(crate) fn removed_occupied(&mut self, len: usize) {
        self.scan_remaining = self.scan_remaining.min(len);
        if self.scan_index >= len {
            self.scan_index = 0;
        }
    }

    pub(crate) fn reserve_capacity(
        &mut self,
        capacity: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        self.queue.reserve_capacity(capacity)?;
        self.retry.reserve_capacity(capacity)
    }

    pub(crate) fn grow_capacity(&mut self, capacity: usize) {
        self.queue.grow_capacity(capacity);
        self.retry.grow_capacity(capacity);
        if let Some(config) = self.config_before.as_mut() {
            *config = config.with_capacity(capacity).expect("已验证容量");
        }
    }

    pub(crate) fn reserved_bytes(&self) -> usize {
        self.queue.reserved_bytes()
            + self.retry.reserved_bytes()
            + self.bounds_diffs.capacity() * std::mem::size_of::<BoundsDiff>()
    }

    pub(crate) fn fallback(&mut self, occupied: usize) {
        self.roots = None;
        self.root_pending = None;
        self.root_diff = None;
        self.bounds_diffs.clear();
        self.diffs.fill(None);
        self.diff_mask = 0;
        self.scan_remaining = occupied;
        self.roots_again = true;
    }

    pub(crate) fn reconfigure(&mut self, previous: Config) {
        self.config_before.get_or_insert(previous);
    }

    fn reset_geometry(&mut self, occupied: usize) {
        self.fallback(occupied);
    }

    fn pending(&self) -> bool {
        !self.queue.is_empty()
            || !self.retry.is_empty()
            || self.roots.is_some()
            || self.roots_again
            || self.root_pending.is_some()
            || self.config_before.is_some()
            || !self.bounds_diffs.is_empty()
            || self.root_diff.is_some()
            || self.diff_mask != 0
            || self.scan_remaining != 0
    }
}

/// 将唯一状态推进一段，输出预算内的异步请求与完整覆盖替换。
///
/// 算法没有第二份资源或覆盖所有者；目标、游标和距离队列也归 `state`。
/// 常规单格移动只启动发生锚点变化的层的扫描线差分；连续移动使某层游标积压，
/// 或发生大跳时，保留节点工作，取消过期几何游标并启用有预算的占用节点核对与最新根枚举。
/// 保底核对不因移动重置位置；目标稳定且容量充足、宿主完成请求与确认后会收敛。
/// LOD0球完整包含有限边界时直接枚举窗口对称差；真实窗口不变时不启动几何工作。
///
/// 优先级仅使用到区块 AABB 的平方距离，按二进制数量级分桶，同桶 FIFO。
/// 出队时重评距离，每八次非空出队轮询下一桶；允许尚未发现候选和移动后的顺序延迟，不保证全局精确最近。
/// 游标、保底扫描与节点协调轮流推进，防止某类工作永久饿死。
///
/// 状态索引采用固定容量的压缩 radix tree，一次查询最多走固定 256 位键的路径，
/// 不进行整表重建。管理开销为 `O(budget.visits)`，常数包含该固定键宽；
/// 桶队列、单步几何与固定八子检查也为有界操作，不排序、不克隆状态或扩容。
/// `State`／`Plan` 的初始化分配及宿主资源工作不在此调用内。
///
/// # 错误
///
/// 输出尚未清空时返回 [`PlanError::NotEmpty`] 且不修改状态；代号耗尽等状态错误
/// 返回 [`PlanError::State`]，此时本批已有输出仍有效，必须处理后才能重用缓冲。
pub fn plan<R>(
    state: &mut State<R>,
    target: Target,
    budget: Budget,
    output: &mut Plan,
) -> Result<Stats, PlanError> {
    if !output.is_empty() {
        return Err(PlanError::NotEmpty);
    }
    update_target(state, target);
    let mut stats = Stats::default();
    for _ in 0..budget.visits {
        let turn = state.schedule.turn;
        state.schedule.turn = (turn + 1) % 8;
        let preferred = match turn {
            0 => Work::Root,
            1 => Work::Diff,
            2 => Work::Scan,
            3 => Work::Retry,
            _ => Work::Node,
        };
        let retry_mask = if state.schedule.retry.is_empty() {
            0
        } else {
            retry_mask(state, budget, output)
        };
        let work = [
            preferred,
            Work::Node,
            Work::Diff,
            Work::Root,
            Work::Scan,
            Work::Retry,
        ]
        .into_iter()
        .find(|work| has_work(state, *work, retry_mask));
        let Some(work) = work else { break };
        stats.visits += 1;
        let result = match work {
            Work::Root => advance_roots(state),
            Work::Diff => advance_diff(state),
            Work::Scan => {
                advance_scan(state);
                Ok(())
            }
            Work::Node => {
                let (handle, bucket) = state.schedule.queue.pop().expect("队列非空");
                let check_distance = !state.schedule.distances_current;
                if state.schedule.queue.is_empty() {
                    state.schedule.distances_current = true;
                }
                advance_node(
                    state,
                    handle,
                    check_distance.then_some(bucket),
                    budget,
                    output,
                    &mut stats,
                )
            }
            Work::Retry => {
                let (handle, _) = state
                    .schedule
                    .retry
                    .pop_mask(retry_mask)
                    .expect("可运行等待项非空");
                advance_node(state, handle, None, budget, output, &mut stats)
            }
        };
        match result {
            Ok(()) => {}
            Err(StateError::Capacity | StateError::RetirementFull | StateError::PendingPatch) => {
                stats.blocked += 1
            }
            Err(error) => return Err(error.into()),
        }
    }
    stats.requests = output.requests.len();
    stats.cancellations = output.cancellations.len();
    stats.switches = output.switches.len();
    stats.pending =
        state.schedule.pending() || state.pending_requests() != 0 || state.pending_patches() != 0;
    stats.capacity_pressure = state.schedule.root_pending.is_some()
        || state.schedule.retry.has_mask(
            (1 << Wait::Node as u8)
                | (1 << Wait::RetireOne as u8)
                | (1 << Wait::RetireFamily as u8),
        );
    Ok(stats)
}

#[derive(Clone, Copy)]
enum Work {
    Root,
    Diff,
    Scan,
    Node,
    Retry,
}

fn has_work<R>(state: &State<R>, work: Work, retry_mask: u128) -> bool {
    match work {
        Work::Root => {
            if let Some(chunk) = state.schedule.root_pending {
                state.available_nodes() != 0
                    || !state
                        .config()
                        .contains_root(chunk, state.target.expect("已有目标"))
            } else {
                state.schedule.roots.is_some() || state.schedule.roots_again
            }
        }
        Work::Diff => {
            (state.schedule.root_pending.is_none()
                && (state.schedule.root_diff.is_some() || !state.schedule.bounds_diffs.is_empty()))
                || state.schedule.diff_mask != 0
        }
        Work::Scan => state.schedule.scan_remaining != 0,
        Work::Node => !state.schedule.queue.is_empty(),
        Work::Retry => state.schedule.retry.has_mask(retry_mask),
    }
}

#[derive(Clone, Copy)]
enum Wait {
    Request,
    Switch,
    Cancel,
    Node,
    RetireOne,
    RetireFamily,
}

fn retry_mask<R>(state: &State<R>, budget: Budget, output: &Plan) -> u128 {
    let conditions = [
        output.requests.len() < budget.requests
            && output.requests.len() < output.requests.capacity(),
        output.switches.len() < budget.switches
            && output.switches.len() < output.switches.capacity(),
        output.cancellations.len() < output.cancellations.capacity(),
        state.available_nodes() != 0,
        state.available_retirements() != 0,
        state.available_retirements() >= 8,
    ];
    conditions
        .into_iter()
        .enumerate()
        .fold(0, |mask, (bit, available)| {
            mask | ((available as u128) << bit)
        })
}

fn update_target<R>(state: &mut State<R>, target: Target) {
    let previous_config = state
        .schedule
        .config_before
        .take()
        .unwrap_or(*state.config());
    let previous = state.target.replace(target);
    if previous != Some(target) {
        state.schedule.distances_current = state.schedule.queue.is_empty();
    }
    let Some(previous) = previous else {
        state.schedule.reset_geometry(state.len());
        state.schedule.roots_again = true;
        return;
    };
    let config = *state.config();
    if previous == target && previous_config == config {
        return;
    }
    state.schedule.capacity_checked = false;
    if let (Some(old), Some(new)) = (
        rectangle(previous_config, previous),
        rectangle(config, target),
    ) {
        if old == new {
            return;
        }
        if state.schedule.root_diff.is_none()
            && state.schedule.diff_mask == 0
            && state.schedule.bounds_diffs.len() < WINDOW_DIFFS
        {
            // 保留有界历史，回转时曾离开/跳过的节点也会收到后续重入事件。
            state
                .schedule
                .bounds_diffs
                .push_back(BoundsDiff::new(old, new));
        } else {
            state.schedule.reset_geometry(state.len());
        }
        return;
    }
    if previous_config != config {
        state.schedule.reset_geometry(state.len());
        return;
    }
    let old = previous.coords();
    let new = target.coords();
    let mut changed = 0_u32;
    for axis in 0..3 {
        let delta = new[axis] - old[axis];
        if !(-1..=1).contains(&delta) {
            state.schedule.fallback(state.len());
            return;
        }
        let crossing = match delta {
            1 => new[axis],
            -1 => old[axis],
            _ => continue,
        };
        if crossing != 0 {
            let lod = crossing.trailing_zeros() + 1;
            if lod < config.lod_count() as u32 {
                changed |= 1 << lod;
            }
        }
    }
    while changed != 0 {
        let lod = changed.trailing_zeros() as usize;
        changed &= changed - 1;
        if state.schedule.diffs[lod].is_none() {
            state.schedule.diffs[lod] = Some(LevelDiffCursor::new(
                config, previous, target, lod as u8, false,
            ));
            state.schedule.diff_mask |= 1 << lod;
        } else {
            state.schedule.fallback(state.len());
            return;
        }
    }
    let top = config.max_lod();
    if config.anchor(previous, top) != config.anchor(target, top) {
        if state.schedule.root_diff.is_none() {
            state.schedule.root_diff = Some(RootDiffCursor::new(config, previous, target));
        } else {
            state.schedule.fallback(state.len());
        }
    }
}

fn rectangle(config: Config, target: Target) -> Option<crate::Bounds> {
    if config.lod_count() != 1 {
        return None;
    }
    let bounds = config.bounds()?;
    for octant in 0..8 {
        let coord = std::array::from_fn(|axis| {
            if octant & (1 << axis) == 0 {
                bounds.minimum()[axis]
            } else {
                bounds.maximum()[axis] - 1
            }
        });
        if !config.contains_root(Chunk::new(coord, 0), target) {
            return None;
        }
    }
    Some(bounds)
}

fn queue_handle<R>(state: &mut State<R>, handle: Handle) {
    if let Some(chunk) = state.chunk(handle) {
        let distance = state
            .config()
            .distance2(chunk, state.target.expect("规划已有目标"));
        state.schedule.wake(handle, distance_bucket(distance));
    }
}

fn retry_handle<R>(state: &mut State<R>, handle: Handle, reason: Wait) {
    state.schedule.queue.discard(handle);
    state.schedule.retry.insert(handle, reason as u8);
}

fn advance_roots<R>(state: &mut State<R>) -> Result<(), StateError> {
    let target = state.target.expect("规划已有目标");
    if let Some(chunk) = state.schedule.root_pending.take() {
        return reserve_root(state, chunk);
    }
    if state.schedule.roots.is_none() {
        state.schedule.roots = Some(RootCursor::new(*state.config(), target));
        state.schedule.roots_again = false;
    }
    match state
        .schedule
        .roots
        .as_mut()
        .expect("已启动根游标")
        .advance()
    {
        CursorStep::Progress => {}
        CursorStep::Done => state.schedule.roots = None,
        CursorStep::Item(chunk) => {
            if state.config().contains_root(chunk, target) {
                reserve_root(state, chunk)?;
            }
        }
    }
    Ok(())
}

fn reserve_root<R>(state: &mut State<R>, chunk: Chunk) -> Result<(), StateError> {
    if !state
        .config()
        .contains_root(chunk, state.target.expect("规划已有目标"))
    {
        return Ok(());
    }
    match state.reserve_root(chunk) {
        Ok(handle) => queue_handle(state, handle),
        Err(StateError::Capacity) => {
            state.schedule.root_pending = Some(chunk);
            if !state.schedule.capacity_checked {
                state.schedule.scan_remaining = state.len();
                state.schedule.capacity_checked = true;
            }
            return Err(StateError::Capacity);
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

fn advance_diff<R>(state: &mut State<R>) -> Result<(), StateError> {
    let target = state.target.expect("规划已有目标");
    if state.schedule.root_pending.is_none() && !state.schedule.bounds_diffs.is_empty() {
        match state
            .schedule
            .bounds_diffs
            .front_mut()
            .expect("已有窗口游标")
            .next()
        {
            Some(RootChange::Enter(chunk) | RootChange::Exit(chunk)) => {
                if state.config().contains_root(chunk, target) {
                    reserve_root(state, chunk)?;
                } else if let Some(handle) = state.find(chunk) {
                    queue_handle(state, handle);
                }
            }
            None => {
                state.schedule.bounds_diffs.pop_front();
            }
        }
        return Ok(());
    }
    let root_turn = state.schedule.diff_root_turn || state.schedule.diff_mask == 0;
    let no_pending_root = state.schedule.root_pending.is_none();
    if let Some(cursor) = state
        .schedule
        .root_diff
        .as_mut()
        .filter(|_| root_turn && no_pending_root)
    {
        state.schedule.diff_root_turn = false;
        match cursor.advance() {
            CursorStep::Progress => {}
            CursorStep::Done => state.schedule.root_diff = None,
            CursorStep::Item(change) => {
                let chunk = match change {
                    RootChange::Enter(chunk) | RootChange::Exit(chunk) => chunk,
                };
                if state.config().contains_root(chunk, target) {
                    reserve_root(state, chunk)?;
                } else if let Some(handle) = state.find(chunk) {
                    queue_handle(state, handle);
                }
            }
        }
        return Ok(());
    }
    state.schedule.diff_root_turn = true;
    let mask = state.schedule.diff_mask;
    let start = state.schedule.diff_turn as u32;
    let following = mask & (u32::MAX << start);
    let lod = if following != 0 { following } else { mask }.trailing_zeros() as usize;
    state.schedule.diff_turn = (lod as u8 + 1) % 31;
    let cursor = state.schedule.diffs[lod]
        .as_mut()
        .expect("活跃层拥有差分游标");
    match cursor.advance() {
        CursorStep::Progress => {}
        CursorStep::Done => {
            state.schedule.diffs[lod] = None;
            state.schedule.diff_mask &= !(1 << lod);
        }
        CursorStep::Item(RootChange::Enter(chunk) | RootChange::Exit(chunk)) => {
            if let Some(handle) = state.find(chunk) {
                queue_handle(state, handle);
            }
        }
    }
    Ok(())
}

fn advance_scan<R>(state: &mut State<R>) {
    let index = state.schedule.scan_index;
    state.schedule.scan_index = (index + 1) % state.len();
    state.schedule.scan_remaining -= 1;
    if let Some(handle) = state.occupied_handle(index) {
        queue_handle(state, handle);
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Empty,
    Requested(crate::Ticket),
    Ready,
    Visible,
    Refreshing(crate::Ticket),
    Replacement,
}

fn phase<R>(value: &ResourcePhase<R>) -> Phase {
    match value {
        ResourcePhase::Empty => Phase::Empty,
        ResourcePhase::Requested(ticket) => Phase::Requested(*ticket),
        ResourcePhase::Ready(_) => Phase::Ready,
        ResourcePhase::Visible(_) => Phase::Visible,
        ResourcePhase::Refreshing { ticket, .. } => Phase::Refreshing(*ticket),
        ResourcePhase::Replacement { .. } => Phase::Replacement,
    }
}

fn root_of(chunk: Chunk, max_lod: u8) -> Chunk {
    let scale = 1_i64 << (max_lod - chunk.lod);
    Chunk::new(
        chunk.coord.map(|coordinate| coordinate.div_euclid(scale)),
        max_lod,
    )
}

fn advance_node<R>(
    state: &mut State<R>,
    handle: Handle,
    bucket: Option<u8>,
    budget: Budget,
    output: &mut Plan,
    stats: &mut Stats,
) -> Result<(), StateError> {
    let Some(node) = state.node(handle) else {
        return Ok(());
    };
    let target = state.target.expect("规划已有目标");
    let config = *state.config();
    if let Some(bucket) = bucket {
        let distance = distance_bucket(config.distance2(node.chunk, target));
        if bucket != distance {
            state.schedule.queue.insert(handle, distance);
            return Ok(());
        }
    }
    if node.pending.is_some() {
        return Ok(());
    }
    let chunk = node.chunk;
    let topology = node.topology;
    let resource = phase(&node.phase);
    let children = node.children;
    let parent = node.parent;
    let invalidated = node.invalidated;
    let outside = !config.contains_root(root_of(chunk, config.max_lod()), target);
    let result = if matches!(resource, Phase::Refreshing(_)) {
        if outside || invalidated {
            cleanup_resource(state, handle, resource, output, stats)
        } else {
            Ok(())
        }
    } else if matches!(resource, Phase::Replacement) {
        if outside || invalidated {
            state.discard_replacement(handle)
        } else {
            emit_patch(state, handle, budget, output, stats, State::prepare_replace)
        }
    } else if !outside && invalidated && !matches!(resource, Phase::Empty) {
        match resource {
            Phase::Visible => emit_refresh(state, handle, budget, output, stats),
            _ => cleanup_resource(state, handle, resource, output, stats),
        }
    } else if outside {
        if topology == Topology::Leaf {
            emit_patch(state, handle, budget, output, stats, State::prepare_remove)
        } else if children.iter().any(Option::is_some) {
            for child in children.into_iter().flatten() {
                queue_handle(state, child);
            }
            cleanup_resource(state, handle, resource, output, stats)
        } else {
            match resource {
                Phase::Empty => {
                    state.detach_empty(handle)?;
                    state.remove_empty(handle)
                }
                _ => cleanup_resource(state, handle, resource, output, stats),
            }
        }
    } else if topology == Topology::Detached {
        let wanted = parent.is_none_or(|parent| {
            state.node(parent).is_some_and(|node| {
                node.topology == Topology::Branch || config.should_split(node.chunk, target)
            })
        });
        if !wanted {
            if children.iter().any(Option::is_some) {
                for child in children.into_iter().flatten() {
                    queue_handle(state, child);
                }
                cleanup_resource(state, handle, resource, output, stats)
            } else if matches!(resource, Phase::Empty) {
                state.remove_empty(handle)
            } else {
                cleanup_resource(state, handle, resource, output, stats)
            }
        } else {
            match resource {
                Phase::Empty => emit_request(state, handle, budget, output, stats),
                Phase::Ready => {
                    let can_activate = parent.is_none_or(|parent| {
                        state
                            .node(parent)
                            .is_some_and(|node| node.topology == Topology::Branch)
                    });
                    if can_activate {
                        emit_patch(
                            state,
                            handle,
                            budget,
                            output,
                            stats,
                            State::prepare_activate,
                        )
                    } else {
                        Ok(())
                    }
                }
                _ => Ok(()),
            }
        }
    } else if config.should_split(chunk, target) {
        if let Some(octant) = children.iter().position(Option::is_none) {
            state.reserve_child(handle, octant).map(|_| ())
        } else if topology == Topology::Leaf {
            let ready = children.into_iter().flatten().all(|child| {
                state.node(child).is_some_and(|node| {
                    node.pending.is_none()
                        && !node.invalidated
                        && matches!(node.phase, ResourcePhase::Ready(_))
                })
            });
            if ready {
                emit_patch(state, handle, budget, output, stats, State::prepare_split)
            } else {
                Ok(())
            }
        } else {
            cleanup_resource(state, handle, resource, output, stats)
        }
    } else if topology == Topology::Branch {
        if let Some(octant) = children.iter().position(Option::is_none) {
            state.reserve_child(handle, octant).map(|_| ())
        } else {
            for child in children.into_iter().flatten() {
                queue_handle(state, child);
            }
            match resource {
                Phase::Empty => emit_request(state, handle, budget, output, stats),
                Phase::Ready => {
                    let leaves = children.into_iter().flatten().all(|child| {
                        state.node(child).is_some_and(|node| {
                            node.topology == Topology::Leaf
                                && node.pending.is_none()
                                && !node.invalidated
                                && matches!(node.phase, ResourcePhase::Visible(_))
                        })
                    });
                    if leaves {
                        emit_patch(state, handle, budget, output, stats, State::prepare_merge)
                    } else {
                        Ok(())
                    }
                }
                _ => Ok(()),
            }
        }
    } else {
        for child in children.into_iter().flatten() {
            queue_handle(state, child);
        }
        Ok(())
    };
    if matches!(result, Err(StateError::Capacity)) {
        retry_handle(state, handle, Wait::Node);
    } else if matches!(result, Err(StateError::RetirementFull)) {
        let family = !outside
            && !config.should_split(chunk, target)
            && topology == Topology::Branch
            && matches!(resource, Phase::Ready);
        retry_handle(
            state,
            handle,
            if family {
                Wait::RetireFamily
            } else {
                Wait::RetireOne
            },
        );
    }
    result
}

fn cleanup_resource<R>(
    state: &mut State<R>,
    handle: Handle,
    phase: Phase,
    output: &mut Plan,
    stats: &mut Stats,
) -> Result<(), StateError> {
    match phase {
        Phase::Ready => state.retire_ready(handle),
        Phase::Requested(ticket) | Phase::Refreshing(ticket) => {
            if output.cancellations.len() == output.cancellations.capacity() {
                stats.blocked += 1;
                retry_handle(state, handle, Wait::Cancel);
                return Ok(());
            }
            state.cancel(ticket)?;
            output.cancellations.push(ticket);
            Ok(())
        }
        Phase::Empty | Phase::Visible => Ok(()),
        Phase::Replacement => state.discard_replacement(handle),
    }
}

fn emit_request<R>(
    state: &mut State<R>,
    handle: Handle,
    budget: Budget,
    output: &mut Plan,
    stats: &mut Stats,
) -> Result<(), StateError> {
    if output.requests.len() >= budget.requests
        || output.requests.len() == output.requests.capacity()
    {
        stats.blocked += 1;
        retry_handle(state, handle, Wait::Request);
        return Ok(());
    }
    output.requests.push(state.request_planned(handle)?);
    Ok(())
}

fn emit_refresh<R>(
    state: &mut State<R>,
    handle: Handle,
    budget: Budget,
    output: &mut Plan,
    stats: &mut Stats,
) -> Result<(), StateError> {
    if output.requests.len() >= budget.requests
        || output.requests.len() == output.requests.capacity()
    {
        stats.blocked += 1;
        retry_handle(state, handle, Wait::Request);
        return Ok(());
    }
    output.requests.push(state.request_refresh_planned(handle)?);
    Ok(())
}

fn emit_patch<R>(
    state: &mut State<R>,
    handle: Handle,
    budget: Budget,
    output: &mut Plan,
    stats: &mut Stats,
    prepare: fn(&mut State<R>, Handle) -> Result<Patch, StateError>,
) -> Result<(), StateError> {
    if output.switches.len() >= budget.switches
        || output.switches.len() == output.switches.capacity()
    {
        stats.blocked += 1;
        retry_handle(state, handle, Wait::Switch);
        return Ok(());
    }
    output.switches.push(prepare(state, handle)?);
    Ok(())
}
