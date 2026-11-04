//! 全进程 Warn/Error 日志；所有 Error 均计数，不按 target 过滤。
//!
//! 显式调用 [`init`]，不创建 GPU。安装结果与错误计数均为全进程状态；已有错误不会
//! 因新一轮检查而清零。测试设施可对 [`errors`] 的快照作 `wrapping_sub` 以归属批次。
#![forbid(unsafe_code)]

use std::sync::{
    OnceLock,
    atomic::{AtomicU32, Ordering},
};

static LOGGER: GpuLog = GpuLog;
static ERRORS: AtomicU32 = AtomicU32::new(0);
static INSTALLED: OnceLock<Result<(), log::SetLoggerError>> = OnceLock::new();

struct GpuLog;

impl log::Log for GpuLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            if record.level() == log::Level::Error {
                ERRORS.fetch_add(1, Ordering::Relaxed);
            }
            eprintln!("[{}] {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

/// 全局日志安装或累计检查失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 已有 logger 或其他安装故障；保留首次安装错误。
    #[error("初始化 GPU 日志失败：{0}")]
    Install(#[source] &'static log::SetLoggerError),
    /// 全进程累计 Error 数量，包含调用前的错误。
    #[error("GPU 日志记录了 {0} 个错误")]
    Reported(u32),
}

/// 全局 logger 只尝试安装一次，后续返回同一结果；与其他 logger 互斥。
pub fn init() -> Result<(), Error> {
    INSTALLED
        .get_or_init(|| {
            log::set_logger(&LOGGER)?;
            log::set_max_level(log::LevelFilter::Warn);
            Ok(())
        })
        .as_ref()
        .map(|_| ())
        .map_err(Error::Install)
}

/// 全进程累计错误数，不重置；差分使用 `wrapping_sub`。
pub fn errors() -> u32 {
    ERRORS.load(Ordering::Relaxed)
}

/// 累计 Error 非零即失败；不消费或重置错误。
pub fn check() -> Result<(), Error> {
    match errors() {
        0 => Ok(()),
        count => Err(Error::Reported(count)),
    }
}

/// 安装 logger 后执行闭包并检查累计错误；日志错误优先于闭包错误。
///
/// GPU 资源须在闭包内释放以捕获析构日志，不能作为成功结果返回仍在使用的资源。
pub fn checked<T, E: From<Error>>(run: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    init()?;
    let result = run();
    check()?;
    result
}

#[cfg(test)]
mod tests {
    use super::{Error, checked};

    #[test]
    fn checked_reports_drop_error() {
        struct Resource;
        impl Drop for Resource {
            fn drop(&mut self) {
                log::error!("析构测试错误");
            }
        }
        let result = checked(|| {
            let _resource = Resource;
            Ok::<_, Error>(())
        });
        assert!(matches!(result, Err(Error::Reported(_))));
    }
}
