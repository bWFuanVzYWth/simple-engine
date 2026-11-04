//! Vulkan 原生计算、光追资源与宿主互操作。
//!
//! [`native`] 接入 wgpu 或外部设备，集中隔离资源、SPIR-V、AS、SBT 和管线的 `unsafe`。
//! [`native::Context`] 唯一拥有 native 资源与录制/在途版本，普通 ID 不自动保活。
//! [`interop`] 维护 wgpu tracker 与同队列完成序号；外部宿主显式报告批次完成。
//! shader 的接口与访存由应用审查，Slang SPIR-V 不经过 Naga 翻译或越界插桩。
//!
//! # 设备与安全边界
//!
//! 宿主实例使用 Vulkan 1.4 loader；普通设备只要求 graphics/compute。光追设备额外要求
//! Vulkan 1.4、`KHR_device_address_commands`、`KHR_acceleration_structure`、`KHR_deferred_host_operations` 和
//! `KHR_ray_tracing_pipeline` 与 `KHR_ray_query`。position fetch 与 OMM 按场景请求；
//! synchronization2 使用核心接口。wgpu 的默认光追入口请求 position fetch 与 OMM。
//! wgpu 的光追便利入口自动检测 EXT SER，支持时启用；compute 不查询 SER。
//! 显式 capabilities 入口只启用请求的能力，SER 不支持时返回 Unsupported。
//! SER 和普通 SPIR-V 的选择由应用依据实际启用能力决定。
//! 开启 position fetch 时，BLAS 使用 `ALLOW_DATA_ACCESS`，hit shader 可读取 AS 内的
//! object-space 顶点，不要求保留原始输入 buffer。支持核心 `pipelineRobustness` 时仅对 native RT 管线
//! 关闭驱动 robustness，否则沿用设备默认；wgpu 管线保持默认保护。
//! AS 输入与 SBT 上传要求 coherent、可映射的 DEVICE_LOCAL 显存，不回退系统 heap；
//! 缺少兼容内存类型时返回 [`Error::Unsupported`]。此约束不保证操作系统永不降级驻留。
//!
//! [`OpacityMicromap`] 只接受精确 2-state bird 数据，不生成或修正覆盖率。BLAS 按 primitive
//! 索引附着 OMM 并保留其所有权；启用 OMM 的设备对应 RT 管线带 micromap flag。
//! BLAS/TLAS 可录制后与追踪按同队列提交，无需 host wait；命令记录保留 input/scratch 至完成。
//! 同步入口保留初始化契约，OMM、BLAS 和 TLAS 由关联场景及在途提交逐级保活。
//! 批构建启用 Context 独占的 input/scratch/backing 分配复用，闲置显存硬上限128MiB；
//! AS handle 仍在真实退役时销毁，闲置分配计入 native memory，Context 析构时全部释放。
//!
//! [`interop::RayTracingPipeline`] 允许 raygen 执行 TraceRay 并调用单层 transport callable。
//! callable 可使用 RayQuery，不执行 TraceRay 或 CallShader；hit/miss 不发起 shader 调用。
//! ray recursion depth 固定为1，使用 Vulkan 默认 stack；RayQuery 不进入 hit/miss shader。
//! 调用图和 payload 兼容性由应用的受控 shader 维护。
//!
//! 本 crate 将 unsafe 集中在明确的资源所有者与受控构建接口内，不反射或证明 shader
//! 的绑定、访存和功能正确性。应用必须保证这些条件，并且不得在 native 录制或在途期间
//! 显式销毁借入的 wgpu 资源。队列交接与提交保活见 [`interop`]。
//!
//! # 验证范围
//!
//! `verify/` 装配生产同源的槽表、资源依赖、真实记录与批次状态、退役分组、闲置顺序和 SDK 恢复核。
//! 契约随所属实现，复杂规范与辅助证明集中在 `src/proof/`；普通 Cargo 构建擦除证明，不运行验证器。
//! `Life` 唯一拥有实际资源、记录、退役组和提交水位；raw payload 通过泛型隔离，未复制运行状态。
//! 资源邻接、Op 分类、完整身份移出及整批状态转换已连接实际存储；合法 Life 操作保持退役完整身份唯一。
//! 通用退役分组另维护条件式唯一性与等待关系。
//! 根游标与真实记录序列、计划及失败原子性已接通；成功计划保留每个现存记录的全部可达依赖。
//! 固定 vstd 的容器规范仍是可信边界。producer 遍历、uses 排序、后续消费者准入、首次等待覆盖的归纳、
//! 实际 rank 销毁及 FFI 尚未完整验证；条件式回收引理不构成端到端安全销毁证明。
//! Vulkan/wgpu 的真实完成、宿主有效期和 Slang/SDK 行为仍须由调用边界维护。
//!
//! 来源：[Vulkan 光追](https://docs.vulkan.org/spec/latest/chapters/raytracing.html)、
//! [AS 构建与同步](https://docs.vulkan.org/spec/latest/chapters/accelstructures.html)、
//! [position fetch](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_ray_tracing_position_fetch.html)、
//! [opacity micromap](https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_opacity_micromap.html)、
//! [pipeline robustness](https://docs.vulkan.org/refpages/latest/refpages/source/VkPipelineRobustnessCreateInfo.html)、
//! [EXT SER](https://docs.vulkan.org/refpages/latest/refpages/source/VK_EXT_ray_tracing_invocation_reorder.html)。

#![deny(unsafe_op_in_unsafe_fn)]
// wgpu 的 Send/Sync 类型图超过默认 trait 求解深度。
#![recursion_limit = "256"]

mod acceleration;
mod buffer;
mod buffer_pool;
mod context;
mod error;
mod graph;
mod handles;
mod health;
mod host;
mod idle;
pub mod interop;
mod lifecycle;
mod micromap;
pub mod native;
mod owner;
mod pipeline_cache;
mod record_ids;
#[cfg(all(feature = "dlss_rr", target_os = "windows"))]
mod recovery;
mod retirement;
mod shader;
#[cfg(all(feature = "dlss_rr", target_os = "windows"))]
pub mod streamline;
#[cfg(all(feature = "dlss_rr", target_os = "windows"))]
mod streamline_interop;
mod submission;

pub use acceleration::{
    AccelerationStructure, BatchBlas, BuildTiming, TriangleBlas, TriangleBlasBatch,
    TriangleBlasInput, TriangleInstance, TriangleSceneBatch,
};
pub(crate) use ash::{khr, vk};
pub use buffer_pool::{AsBuildMemory, AsPoolStats};
pub(crate) use context::{Device, Instance, PhysicalDevice};
pub use error::Error;
pub use micromap::{OPACITY_FULLY_OPAQUE, OpacityMap, OpacityMicromap};
pub(crate) use shader::ShaderModule;
pub use submission::FailureKind as DeviceFailure;

#[cfg(test)]
mod tests {
    #[test]
    fn spirv_decode_preserves_words() {
        let code: [u32; 5] = [0x0723_0203, 0x0001_0300, 0x0008_0001, 7, 0];
        let bytes: Vec<_> = code.iter().flat_map(|word| word.to_le_bytes()).collect();
        assert_eq!(super::shader::decode_spirv(&bytes).unwrap(), code);
    }
}
