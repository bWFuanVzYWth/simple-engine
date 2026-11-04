//! AS GPU 回归的 validation 错误锁存；这些用例必须串行运行。
use std::sync::{
    Once,
    atomic::{AtomicBool, Ordering},
};

struct Logger;
static LOGGER: Logger = Logger;
static INIT: Once = Once::new();
static FAILED: AtomicBool = AtomicBool::new(false);

impl log::Log for Logger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!(
                "[{}] {}: {}",
                record.level(),
                record.target(),
                record.args()
            );
            if record.level() == log::Level::Error {
                FAILED.store(true, Ordering::Relaxed);
            }
        }
    }

    fn flush(&self) {}
}

pub(super) fn start() {
    INIT.call_once(|| {
        log::set_logger(&LOGGER).expect("AS回归须独占测试进程的logger");
        log::set_max_level(log::LevelFilter::Warn);
    });
    FAILED.store(false, Ordering::Relaxed);
}

pub(super) fn finish() -> Result<(), crate::Error> {
    if FAILED.load(Ordering::Relaxed) {
        Err(crate::Error::InvalidState(
            "AS回归记录了validation或设备错误",
        ))
    } else {
        Ok(())
    }
}
