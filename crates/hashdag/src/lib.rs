//! 64 叉持久化体素 DAG；资源由一个所有者持有，读取共享、写入独占。
//!
//! 每轴 20 bit 的 [`ZOrder20x3`] 路径经过十层节点，每层消费两个 Morton digit。
//! 叶保存完整 `u32`，不解释材料、空气或世界坐标。相同层级的节点按完整内容共享，
//! 同值叶区域折叠；已发布节点不可变，修改沿路径构建并发布新根。
//!
//! [`HashDag`] 拥有节点池、根槽与可复用写缓冲。复制 [`RootId`] 不增加引用，
//! [`HashDag::fork_root`] 才创建独立根，调用者须用 [`HashDag::release_root`] 释放。
//! 默认根由 DAG 持有到销毁。节点位置会复用，不能作为跨编辑的永久内容标识。
//!
//! 批写的输入排序、区域对齐和重叠约束在接收边界检查。健康 DAG 上可恢复构建失败
//! 保留旧根；已经增长的容量不会回滚。内核不调度线程，应用按块共享只读输入并独占输出。
//!
//! # 内存与层级
//!
//! 每个原生节点的 64 个 payload 为 256 B，独立指针 mask 为 8 B；RC、去重表和空闲槽
//! 为 CPU 维护数据。原生层 `level=0..10` 的节点边长为 `4^(level+1)`；区域接口的
//! `lod=0..=20` 表示边长 `2^lod`，二者不可混用。编号在回收前稳定，释放不缩短物理数组。
//!
//! 批读复用 Morton 空间前缀，输出由调用者持有。批写以深度有界的候选帧推进，dense
//! 每层保留 64 个子项，不保存完整归约层；新唯一节点的构造持有仍随节点数增长。
//! 池和写缓冲保留高水位容量，统计见 [`HashDag::retained_bytes`]；世界销毁时统一释放。
//! 物理数组 ABI 不代表跨设备零拷贝，本 crate 不管理 GPU、上传日志或应用版本。
//! CPU 射线支持 [`HashDag::raycast_batch`]；结果由调用者持有，回调按射线顺序执行。
//! [`NodeDiff`] 精确计算两个保活根的可达物理节点集合差，消费者复用遍历位图、输出及容量；
//! DAG 不增加变更日志或 dirty 状态。根子项自身的变化由消费者另行处理。
//!
//! [`LocalBuilder`] 可在调用方调度的 worker 中构建独占 [`LocalTree`]，再以
//! [`HashDag::set_built_regions`] 批量导入；局部唯一节点只按层翻译一次，不重读 dense。
//! worker 帧、结果槽池和目标翻译表各自复用；该路径没有全局写锁或内核内部线程调度。
//!
//! # 形式化契约
//!
//! 首批维护下述五项性质，以生产同源接口和所有者不变量连接证明；完整检查见
//! `scripts/verify_hashdag.ps1`。重点是共享节点的唯一化、发布与回收，以及精确差分，
//! 不以逐方法覆盖率或重复装配的证明单元数计覆盖。
//!
//! ## 统一语义
//!
//! 令 `m` 为合法的 60-bit Morton 路径，`h` 为剩余原生高度，`E_D(c, h, m)` 为 DAG
//! `D` 中子项 `c` 的体素值。有效域为 `0 <= h <= 10`；Ptr 只在 `h > 0` 时有效，
//! 且须指向 `pool[h-1]` 的存活节点。`h = 0` 时只能为叶，根的高度为 10：
//!
//! ```text
//! E_D(Leaf(v), h, m) = v
//! E_D(Ptr(i), h, m) = E_D(pool[h-1][i].child((m >> (6*(h-1))) & 63), h-1, m)
//! V_D(root, m) = E_D(root.child, 10, m)
//! A_l(m) = floor(m / 2^(3*l)) * 2^(3*l)
//! Q_l(m) = [A_l(m), A_l(m) + 2^(3*l)), 0 <= l <= 20
//! ```
//!
//! `Q_l` 对应每轴向下对齐、边长 `2^l` 的立方体，奇数 LOD 也使用同一定义。
//! Morton 位语义为 `m[3*j] = x[j]`、`m[3*j+1] = y[j]`、`m[3*j+2] = z[j]`，
//! `0 <= j < 20`；编码与解码双向互逆，越界构造返回 `None`。
//! `E_D(c, h, m)` 只依赖低 `6*h` 位，子项语义等价比较全部局部路径 `0 <= m < 2^(6*h)`。
//! `Leaf(v)` 的全部 `u32` 位型均合法，payload 的数值不能代替独立 mask 判定 Ptr。
//!
//! ## 首批性质
//!
//! 1. **结构合法与规范化。** 各池的并行数组同长，存活节点与空闲槽互斥；
//!    已发布 payload/mask 不变，Ptr 只指向下一层已分配节点，最低层没有 Ptr。
//!    64 个同值 Leaf 当且仅当可折叠为该 Leaf；相同非均匀 Ptr 不折叠空间层级。
//!    同层完整 payload 加 mask 唯一化，任意哈希碰撞不能合并不同节点。
//!    在同一健康 DAG、同一有效高度、两个子项均有效并保活时，子项相等当且仅当全部体素值相等；
//!    此结论支持快照内容比较和相等子树剪枝。
//!
//! 2. **结构引用守恒与回收。** 在全局 [`HashDag`] 的稳定公开调用边界：
//!    `RC(n) = 指向 n 的活根数 + 活的唯一物理父节点中指向 n 的 Ptr 槽数`。
//!    同父重复 64 条边计 64，不能按子节点集合或展开路径数计数。构建期间另有每个
//!    首次 New 的一份构造持有；发布交接和部分认领须按阶段记账，稳定式不强加于每条语句。
//!    Existing 不重复认领子边，New 完整认领后才发布；认领失败精确撤销本次前缀。
//!    回收恰好删除最后一份持有消失的节点，共享子树保留；先移出唯一化表再复用槽，
//!    无执行递归、回收不分配，工作栈不超过 `1 + 63*10 = 631` 项。
//!    内部 Pool 可暂存零引用未认领节点；[`LocalTree`] 保留构造持有至整体清空，
//!    二者不能直接套用全局稳定式。scratch 与翻译表的借用不计结构持有。
//!
//! 3. **事务写语义与快照隔离。** 健康 DAG 成功写入后，覆盖区内等于该批输入，区外
//!    保持 `V_D`；其它活根的 Child、内容和身份不变。单点批按最后一条同路径记录取值；
//!    uniform、dense 和 built 区域使用 `Q_l`，dense 为局部 Morton 顺序。
//!    健康 DAG 的可恢复输入／资源失败保留原根与既有快照，清空本次构造持有，不遗留
//!    不可达活节点。容量、物理槽数量、已消耗 generation 及哈希表布局允许变化。
//!    发布后的内部结构损坏或清理失败不承诺回滚；健康不变量排除了这些失败路径，
//!    不能把结论写成任意损坏状态下所有 `Err` 均恢复原字节。
//!
//! 4. **根身份与内容身份。** 对公开 API 合法发行的 [`RootId`]（包括其复制、外来和
//!    过期句柄），fork 成功才增加独立根持有；复制句柄不保活。释放成功永久使旧句柄
//!    失效，同槽复用不复活旧代次；默认根不可释放，代次饱和时拒绝释放且根仍可读写。
//!    [`NodeIdx`] 仅是层内位置，不能承诺旧编号在复用后仍被拒绝；跨编辑缓存须使用
//!    [`NodeIdentity`]、层级和槽号，generation 耗尽不得回绕或破坏既有内容。
//!
//! 5. **精确增量上传集合。** 令 `R_D(r)` 为根可达的唯一 `(level, index)` 集合，
//!    [`NodeDiff::between`] 返回 `sort(R_D(after) ∖ R_D(before))`，严格不重复。
//!    旧根在其它空间位置已引用的节点也必须排除，叶根变化由消费者更新 root 描述。
//!    先标记旧根的全部可达节点，再遍历新根；命中旧集合可跳过整个子图。
//!    每次实际遍历前按当前物理槽重置位图，失败清空输出，不改变 DAG 的真实引用计数或保活节点。
//!    根内容相等或新根为叶时可直接返回空集，不依赖跨调用身份缓存。
//!
//! ## 来源、后续范围与可信边界
//!
//! [`LocalBuilder::build`] 成功表示全部 dense 值；校验失败保留旧结果，开始构建后的
//! 资源失败则使结果失效。导入映射须保持语义且所有目标 Ptr 只指目标池，丢弃／重建
//! 局部树不影响已导入内容。奇数 LOD 的八段须连续并按序消费，首段先建立对应翻译表；
//! 证明覆盖公开输入校验向私有 dense 内核传递精确长度，以及归约和导入过程。
//!
//! 批读和区域／平面展开属于后续范围：应逐项等于 `V_D`；`Uniform` 当且仅当区域全部同值，Mixed 为其反面。
//! 读接口的 root、排序和形状错误须先于输出修改；内部结构错误可留下部分输出。
//! 各入口保持已有错误优先级、对齐规则及 `usize` 长度可表示性，不新增公共前置条件。
//!
//! 射线单独分阶段：先以 Q40 整数时间序与 X/Y/Z 等时规则验证 DDA、均匀叶跳过及
//! 有界推进，再审查浮点准备与真实调用域；不直接宣称等于任意实数射线。
//! 回调顺序与提前终止是契约，任意用户回调的有限返回／无 panic 不能由 DAG 单独保证。
//!
//! hashbrown 按可信容器处理；完整内容判等、哈希一致性、入池及引用变化仍由同源代码验证。
//! 标准库、SIMD、分配、原子线性化及静态对象身份只在最小适配边界声明可信条件。
//! owner 的 checked 递增、饱和与唯一票据由同源发行核证明；票据和历史完全擦除。
//! 契约针对由公开构造／修改接口维持的健康所有者及正常返回，不包含整体 unwind 恢复，
//! 也不证明标准库、allocator、GPU 或上传调度的实现。资源成本另作实测：候选帧按深度有界，但构造记录、
//! 翻译表随节点数增长，差分位图还覆盖空闲物理槽；遍历工作集按实现评估，不能只统计输出。
//! 独立 dense／可达性／引用审计、故障测试与性能对照仍作为不同维度的回归证据。
//!
//! ```
//! use hash_dag::{HashDag, SetCell, ZOrder20x3};
//!
//! let mut dag = HashDag::new()?;
//! let root = dag.root();
//! let path = ZOrder20x3::encode(3, 5, 7).unwrap();
//! dag.set_cells(root, &[SetCell { path, data: u32::MAX }])?;
//! let snapshot = dag.fork_root(root)?;
//! dag.set_cells(root, &[SetCell { path, data: 0 }])?;
//! assert_eq!(dag.value_at(snapshot, path)?, u32::MAX);
//! dag.release_root(snapshot)?;
//! # Ok::<(), hash_dag::Error>(())
//! ```

#![feature(portable_simd)]
#![cfg_attr(not(verus_only), feature(stmt_expr_attributes))]
#![forbid(unsafe_code)]

mod diff;
#[cfg(not(verus_only))]
mod diff_ops;
mod error;
mod hashdag;
#[cfg(not(verus_only))]
mod identity;
#[cfg(not(verus_only))]
mod identity_ops;
mod layers;
mod layout;
mod morton;
mod node;
mod pool;
mod raycast;
mod rc;
mod read;
mod roots;
#[cfg(not(verus_only))]
mod simd;
#[cfg(not(verus_only))]
mod table_ops;
#[cfg(not(verus_only))]
mod vec_ops;
mod wide;
mod write;

#[cfg(verus_only)]
#[path = "proof/balance.rs"]
pub(crate) mod balance;
#[cfg(verus_only)]
#[path = "proof/canonical.rs"]
pub(crate) mod canonical;
#[cfg(verus_only)]
#[path = "proof/commit.rs"]
pub(crate) mod commit_proof;
#[cfg(verus_only)]
#[path = "proof/state.rs"]
pub(crate) mod state;

#[cfg(test)]
mod test_util;

pub use diff::{NodeAddress, NodeDiff};
pub use hashdag::{Error, HashDag, RootId};
pub use morton::ZOrder20x3;
pub use node::{Child, NodeIdx, NodeMask, NodeValues};
pub use raycast::{
    AbortReason, Axis, BlockedReason, HitBehavior, InvalidInputReason, MissReason, Ray, RaySample,
    RaycastResult, SignedVoxelCoord, VoxelCoord, VoxelHit,
};
pub use read::{NodeIdentity, SubtreeState};
pub use write::built::{BuiltRegion, LocalBuilder, LocalTree};
pub use write::dense_chunks::DenseRegion;
pub use write::fill_uniform::UniformFill;
pub use write::set_cells::SetCell;
