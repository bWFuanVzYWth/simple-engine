//! 预算驱动的三维区块生命周期；不创建线程，不绑定建网或 GPU 后端。
//!
//! [`State<R>`] 是资源、当前覆盖 cut、异步请求和暂停位置的唯一所有者；
//! [`plan`] 只执行有预算的协调，输出不持资源的 [`Build`] 与 [`Patch`]。
//! 没有第二份已加载或可见集合，也不要求 `R: Clone`。本 crate 无递归、引用计数或 `unsafe`。
//!
//! # 空间与优先级
//!
//! LOD0 边长由宿主决定；若为 64m，LOD1 为 128m，之后逐层翻倍。
//! [`Target`] 使用 LOD0 整数坐标，各层球心独立吸附到最近本级网格。
//! 目标以球和 AABB 相交决定完整父块细化，根球额外保留外围覆盖。
//! 原始观察位置到 AABB 的平方距离决定常规优先级，采用有界数量级桶，允许近似顺序。
//! 每八次出队轮询下一非空桶，避免持续近处工作永久饿死已发现的远处工作。
//!
//! # 异步与覆盖
//!
//! 宿主将请求交给自己的调度器，构建完成后把 owned 资源交回 [`State::complete`]。
//! 父叶始终保留，直到八个子资源完成并准备好最终渲染覆盖、GPU资源和接缝；
//! [`State::publish`] 才整体切换。合并按相反方向进行；编辑刷新也直到新资源发布才替换。
//! 未完成、失败、取消、GPU准备失败或容量压力都不提前撤掉旧覆盖。未知/未加载源不能
//! 伪装成真实空几何提交，只有确知真实为空的输入才允许空资源；源语义由宿主负责。
//! 管理器保证替换不产生父子重叠或半家族空洞，几何接缝由宿主
//! 根据 [`Change`] 准备，邻接 LOD 的平衡或具体 stitching 方法不在通用资源类型中假定。
//! 家族锁不锁相邻区块；宿主须按同一发布边界的最终邻接准备所有受影响接缝。
//! 延迟确认期间邻接变化时须重新准备，不能复用旧邻接的结果。只支持相邻一级的
//! stitching 还需要宿主适配层约束当前覆盖的 2:1 平衡。
//! 根退出需求域后可逐叶回收；新区域和瞬移需要加载，不能凭空保证立即覆盖。
//!
//! 编辑通过 [`State::invalidate`] 标记，规划按请求额度构建新资源。旧可见资源一直保留到
//! [`Change::Replace`] 发布；反复编辑会取消过期请求，晚到结果仍交回宿主。
//! split/merge锁定家族中任一父/子节点的编辑均使整项变化不可发布，拒绝后旧覆盖仍保留。
//! [`State::prepared_visible`] 和 [`State::prepared_face_neighbors`] 直接投影所有待确认变化，
//! 宿主无需另建资源所有者；提交前以 [`State::validate_patch`] 检查计划仍有效。
//! 面邻居游标逐节点枚举任意层差的叶与 AIR 缺口，覆盖变化时返回 [`NeighborStep::Changed`]。
//!
//! [`Bounds`] 使用 LOD0 整数半开单位；根分配和后续细分受其约束，完整八子家族保持不变，
//! 边界外的叶由宿主提供空资源。它不负责裁剪网格。半径和边界可通过 [`State::reconfigure`]
//! 修改，保留已有资源和请求，按后续预算协调；层数固定，容量可显式帧外扩容。
//!
//! 原子发布只改变之后使用的可见集合；已被旧提交引用的资源仍由状态持有，
//! 在最后一次使用的完成值到达后才可由 [`State::take_retired`] 取回。
//! 相机移动不会全局作废仍可使用的 ticket。未被接受的完成值通过 [`Rejected<R>`]
//! 原样返还，宿主负责其最终处理。状态和退役队列均有容量上限，需保留转换余量。
//! 不再需要的在途请求通过 [`Plan::cancellations`] 通知宿主；取消不了的外部任务
//! 仍可交回结果，状态会拒绝旧 ticket 并返还资源。
//!
//! # 帧预算
//!
//! [`Budget::visits`] 包含无输出检查；不会先枚举完整差分再截取前几块。
//! 初始化和瞬移同样可暂停。LOD0完整有限窗口按不相交条带直接处理对称差；
//! 重复配置与目标先合并，真实窗口不变时不重置游标。连续移动造成积压时会保守核对已占用节点，总工作可能大于
//! 最终差分，但单次调用保持预算界。分配在 [`State::new`] 和 [`Plan::with_capacity`]
//! 完成；规划内不扩容。宿主可据 [`Stats::capacity_pressure`] 在帧外调用
//! [`State::grow_capacity`]，不重建状态或复制资源。完成、发布、回收各处理一个有界事务，由宿主另行限制调用数。
//! 输出或容量不足的工作按阻塞原因暂停，条件不可改善时不会继续消耗本批访问额度。
//! 增量视图使用 [`State::patch_resources`]，无需每帧遍历全部覆盖。
//!
//! ```
//! use chunk_manager::{Budget, Config, Plan, State, Target, plan};
//!
//! let mut state = State::<String>::new(Config::new(3, 4, 20_000)?);
//! let mut batch = Plan::with_capacity(8, 4);
//! let target = Target::new([0, 0, 0])?;
//! let stats = plan(&mut state, target,
//!     Budget { visits: 64, requests: 8, switches: 4 }, &mut batch)?;
//! for build in batch.requests() {
//!     // 示例同步完成；实际应用可将 ticket 和坐标交给外部异步调度器。
//!     state.complete(build.ticket, format!("{:?}", build.chunk)).unwrap();
//! }
//! for patch in batch.switches() {
//!     // 先准备接缝与绘制集合；publish 成功后一起启用。此处没有 GPU，完成值为 0。
//!     state.publish(patch.token, 0)?;
//! }
//! batch.clear();
//! assert!(stats.visits <= 64);
//! // 每次最多取回一个资源，宿主也应限制回收次数。
//! drop(state.take_retired(0));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

mod bounds_diff;
mod changes;
mod geometry;
mod index;
mod neighbors;
mod planner;
mod queue;
mod state;

pub use changes::{PatchResources, VisibleChange};
pub use geometry::{
    Bounds, Chunk, Config, ConfigError, CursorStep, LevelDiffCursor, RootChange, RootCursor,
    RootDiffCursor, Target,
};
pub use neighbors::{Face, FaceNeighbor, FaceNeighbors, NeighborStep};
pub use planner::{Budget, Plan, PlanError, Stats, plan};
pub use state::{
    Build, Change, GrowthError, Handle, Patch, PatchToken, Rejected, State, StateError, Ticket,
};
