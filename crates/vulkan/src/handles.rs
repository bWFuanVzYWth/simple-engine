//! 资源与记录身份；复制标识不延长底层资源寿命。

use crate::owner::Key;
#[cfg(verus_only)]
use vstd::prelude::*;

/// Context 中的 buffer 版本标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct Buffer {
    pub(crate) key: Key,
    pub(crate) len: u64,
    pub(crate) host_visible: bool,
}

/// 唯一 Context 中的图像标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct Image {
    pub(crate) key: Key,
    pub(crate) extent: [u32; 3],
}

/// Context 中的 sampler 标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct Sampler {
    pub(crate) key: Key,
}

/// 不可变 descriptor 版本标识；资源在唯一 Context 中保存。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct Bindings {
    pub(crate) key: Key,
}

/// 光追 pipeline 的不可变 program/binding 组合标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct RayTracingPipeline {
    pub(crate) key: Key,
}

/// Compute program/binding 组合标识。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct ComputePipeline {
    pub(crate) key: Key,
}

/// Context 中唯一保存的 BLAS 标识；复制标识不保活资源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct TriangleBlas {
    pub(crate) key: Key,
}

/// Context 中唯一保存的 TLAS 标识；依赖的 BLAS/OMM 由版本根维护。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct AccelerationStructure {
    pub(crate) key: Key,
}

/// 同步构建完成的 2-state OMM；输入和 scratch 已可释放。
///
/// Copy ID 不保活资源。Context 按 TLAS→BLAS→OMM 版本依赖保留真实数据。
/// KHR OMM 使用 AS handle，退役时先销毁 handle 再释放 backing；BLAS 始终引用 OMM。
/// 不得与其他设备的 BLAS 混用，也不启用 lossy 压缩。
/// <https://docs.vulkan.org/refpages/latest/refpages/source/VK_KHR_opacity_micromap.html>
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct OpacityMicromap {
    pub(crate) key: Key,
    pub(crate) count: usize,
}

/// Context 中的 timestamp pool 身份；复制不保活，collect 时须保留为资源根。
/// 同一 pool 同时只允许一个待执行或在途访问记录，完成通知收取后才可重新 reset。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct TimestampQueries {
    pub(crate) key: Key,
    pub(crate) count: u32,
}

/// collect 的已发布资源根；由相应 native ID 转换。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub struct Resource(pub(crate) Key);

/// 与资源键分离的完整记录身份。
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct RecordId(pub(crate) Key);

#[cfg_attr(verus_only, verus_verify)]
impl RecordId {
    #[cfg_attr(verus_only, verus_spec(id => ensures id.0 == key))]
    pub(crate) const fn new(key: Key) -> Self {
        Self(key)
    }

    #[cfg_attr(verus_only, verus_spec(key => ensures key == self.0))]
    pub(crate) const fn key(self) -> Key {
        self.0
    }

    #[cfg_attr(verus_only, verus_spec(same => ensures same == (self == other)))]
    pub(crate) const fn same(self, other: Self) -> bool {
        self.0.same(other.0)
    }
}

/// 命令对 native buffer 或 image 的完整资源访问声明；不区分读写和子范围。
///
/// 登记后同时保活该资源，直到取消或收取完成通知前拒绝其 host 读写。
/// 该声明不推断 shader/SDK 的真实访问，也不覆盖借入的 wgpu/raw 资源。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(verus_only, verus_verify)]
pub enum ResourceAccess {
    /// 完整 buffer，不区分读写和子范围。
    Buffer(Buffer),
    /// 完整 image，不区分读写和子范围。
    Image(Image),
}

#[cfg_attr(verus_only, verus_verify)]
impl ResourceAccess {
    #[cfg_attr(verus_only, verus_spec(key => ensures key == match self {
        Self::Buffer(buffer) => buffer.key, Self::Image(image) => image.key,
    }))]
    pub(crate) const fn key(self) -> Key {
        match self {
            Self::Buffer(buffer) => buffer.key,
            Self::Image(image) => image.key,
        }
    }
}

impl From<Buffer> for ResourceAccess {
    fn from(value: Buffer) -> Self {
        Self::Buffer(value)
    }
}
impl From<Image> for ResourceAccess {
    fn from(value: Image) -> Self {
        Self::Image(value)
    }
}

/// 外部描述符类型；图像使用 GENERAL。
#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub enum ExternalKind {
    /// storage buffer。
    StorageBuffer {
        /// 完整可访问字节数。
        len: u64,
    },
    /// storage image view。
    StorageImage,
    /// sampled image view。
    SampledImage,
    /// acceleration structure。
    Acceleration,
}

/// 经 raw 边界借入的宿主描述符；不延长真实宿主寿命。
#[derive(Clone, Copy)]
#[cfg_attr(verus_only, verus_verify)]
pub struct ExternalBinding {
    pub(crate) owner: u64,
    pub(crate) handle: u64,
    pub(crate) kind: ExternalKind,
}

macro_rules! roots {
    ($($ty:ty),*) => { $(impl From<$ty> for Resource {
        fn from(value: $ty) -> Self { Self(value.key) }
    })* };
}
roots!(
    Buffer,
    Image,
    Sampler,
    Bindings,
    RayTracingPipeline,
    ComputePipeline,
    TriangleBlas,
    AccelerationStructure,
    OpacityMicromap,
    TimestampQueries
);
