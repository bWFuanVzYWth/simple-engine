//! GPU 单元/属性测试与测量设施；native 直接执行生产 Slang SPIR-V。
//!
//! wrapper 只编排生产 shader 的输入输出，Rust 对照不能替代 GPU 检验。环境和管线跨用例
//! 复用，输入按批派发；每次执行独占 IO、提交与错误归属。用例按设备能力保留 ignored，
//! 由 [`run`] 显式清理共享环境；单独创建的环境由 [`Gpu::finish`] 检查析构结果。
//! 性能样本用于分析，不设置回归门槛；准备、GPU 执行与主机整链分别计时。
#![forbid(unsafe_code)]
#![recursion_limit = "256"]

mod gpu;
pub mod input;
pub mod record;

pub use gpu::{Capability, Error, Gpu, Sample, Shader, Stats, with_shared};
pub use input::{DEFAULT_SEED, PropertyRng, SeedError, parse_seed, unit_f32};
pub use libtest_mimic::Trial;
pub use record::Failure;

/// 保留 libtest 的 ignore/filter 入口；返回进程状态前显式释放共享 GPU 环境。
pub fn run(tests: Vec<Trial>) -> std::process::ExitCode {
    let result = libtest_mimic::run(&libtest_mimic::Arguments::from_args(), tests);
    if let Err(error) = shutdown_shared() {
        eprintln!("GPU 清理失败：{error}");
        return std::process::ExitCode::FAILURE;
    }
    result.exit_code()
}

/// 取出并销毁所有共享环境；报告等待、validation 和析构错误。
pub fn shutdown_shared() -> Result<(), Error> {
    gpu::shutdown_shared()
}
