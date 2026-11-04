//! 可恢复输入、生命周期与资源错误。

use crate::NodeIdx;
#[cfg(verus_only)]
use vstd::prelude::*;

#[cfg(verus_only)]
verus! {
impl vstd::std_specs::convert::FromSpecImpl<std::collections::TryReserveError> for Error {
    open spec fn obeys_from_spec() -> bool { true }
    open spec fn from_spec(source: std::collections::TryReserveError) -> Self { Self::Allocation(source) }
}
impl vstd::std_specs::convert::FromSpecImpl<hashbrown::TryReserveError> for Error {
    open spec fn obeys_from_spec() -> bool { true }
    open spec fn from_spec(source: hashbrown::TryReserveError) -> Self { Self::HashAllocation(source) }
}
}

/// 输入、生命周期、资源增长或内部节点引用错误。
#[derive(Debug)]
#[cfg_attr(not(verus_only), derive(thiserror::Error))]
#[cfg_attr(verus_only, verus_verify)]
pub enum Error {
    /// 局部构建结果尚未完成，或最近一次资源失败已使其失效。
    #[cfg_attr(not(verus_only), error("局部子树尚未构建成功"))]
    SubtreeNotBuilt,
    /// 构建帧缺失，表示内部结构不一致。
    #[cfg_attr(not(verus_only), error("构建帧缺失"))]
    BuildFrameMissing,
    /// 默认 root 由 DAG 持有，不能单独释放。
    #[cfg_attr(not(verus_only), error("不能释放默认 root"))]
    CannotReleaseDefaultRoot,
    /// dense 值数量与区域 LOD 不一致。
    #[cfg_attr(not(verus_only), error("dense 值数量与区域不一致"))]
    InvalidDenseValues,
    /// 输入长度、顺序、对齐或区域关系不满足接口契约。
    #[cfg_attr(not(verus_only), error("输入不满足接口约束"))]
    InvalidInput,
    /// LOD 超出接口支持范围。
    #[cfg_attr(not(verus_only), error("不支持的 LOD：{0}"))]
    InvalidLod(u8),
    /// 内部指针指向缺失节点。
    #[cfg_attr(not(verus_only), error("节点缺失：{0:?}"))]
    NodeMissing(NodeIdx),
    /// 层内 u32 节点编号空间耗尽。
    #[cfg_attr(not(verus_only), error("节点编号空间耗尽"))]
    IndexExhausted,
    /// 节点引用数量超过 usize 范围。
    #[cfg_attr(not(verus_only), error("节点引用计数溢出"))]
    RefCountOverflow,
    /// 容量或批量长度计算超过 usize 范围。
    #[cfg_attr(not(verus_only), error("容量计算溢出"))]
    CapacityOverflow,
    /// root 来自其他 DAG 实例。
    #[cfg_attr(not(verus_only), error("root 不属于此 DAG"))]
    WrongOwner,
    /// 进程内不重复的实例身份空间耗尽。
    #[cfg_attr(not(verus_only), error("DAG 实例身份空间耗尽"))]
    IdentityExhausted,
    /// root 槽 generation 饱和；该 root 仍然存活。
    #[cfg_attr(not(verus_only), error("root generation 已饱和"))]
    RootGenerationExhausted,
    /// 节点内容身份空间耗尽；已有节点和根保持有效。
    #[cfg_attr(not(verus_only), error("节点 generation 已饱和"))]
    NodeGenerationExhausted,
    /// root 已释放或槽不存在。
    #[cfg_attr(not(verus_only), error("root 已失效"))]
    RootMissing,
    /// 顺序容器分配失败，保留标准库错误来源。
    #[cfg_attr(not(verus_only), error("容器容量申请失败：{0}"))]
    Allocation(#[cfg_attr(not(verus_only), source)] std::collections::TryReserveError),
    /// 去重表分配失败，保留 hashbrown 错误来源。
    #[cfg_attr(not(verus_only), error("去重表容量申请失败：{0}"))]
    HashAllocation(#[cfg_attr(not(verus_only), source)] hashbrown::TryReserveError),
}

#[cfg_attr(verus_only, verus_verify)]
impl From<std::collections::TryReserveError> for Error {
    #[cfg_attr(verus_only, verus_spec(result => ensures result == Self::Allocation(source)))]
    fn from(source: std::collections::TryReserveError) -> Self {
        Self::Allocation(source)
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl From<hashbrown::TryReserveError> for Error {
    #[cfg_attr(verus_only, verus_spec(result => ensures result == Self::HashAllocation(source)))]
    fn from(source: hashbrown::TryReserveError) -> Self {
        Self::HashAllocation(source)
    }
}
