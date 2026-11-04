//! 生产 Slang 模块的批量属性测试与定向负载。
#![forbid(unsafe_code)]

pub mod input;
pub mod run;

/// 实际执行的 Slang 版本输出。
pub const SLANG_VERSION: &str = env!("SLANG_VERSION");
/// shader、宿主源码、构建工具与编译器的整体身份。
pub const SOURCE_BLAKE3: &str = env!("SOURCE_BLAKE3");
/// shader 构建输入与实际 recipe 参数的身份。
pub const SHADER_BUILD_BLAKE3: &str = env!("SHADER_BUILD_BLAKE3");
/// 全部 native shader 入口与实际 SPIR-V 字节的身份。
pub const SPIRV_BLAKE3: &str = env!("SPIRV_BLAKE3");
