use std::{
    fmt,
    sync::{Condvar, Mutex, MutexGuard},
    time::Instant,
};

struct State<T> {
    pending: Option<T>,
    publisher_open: bool,
    consumer_open: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 单槽交接的唯一所有者；两端借用它，必须先停止并 join 消费线程再释放。
///
/// 发布移动完整值，只覆盖未取走的旧值；槽位与关闭状态共用短临界区，值在锁外析构。
/// 不克隆载荷或调用应用回调；`T: Send` 即可跨 scoped 线程交接，不要求 `T: Sync`。
/// 锁中毒时恢复仅含槽位移动和布尔修改的内部状态；不保证实时上限或公平性。
pub struct Latest<T> {
    shared: Shared<T>,
}

impl<T> Latest<T> {
    /// 建立空槽，不分配；`split` 才签发两端。
    pub const fn new() -> Self {
        Self {
            shared: Shared {
                state: Mutex::new(State {
                    pending: None,
                    publisher_open: false,
                    consumer_open: false,
                }),
                changed: Condvar::new(),
            },
        }
    }

    /// 独占签发不可克隆的两端；此前两端和借用必须已结束。
    pub fn split(&mut self) -> (Publisher<'_, T>, Consumer<'_, T>) {
        let state = self
            .shared
            .state
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.publisher_open = true;
        state.consumer_open = true;
        (
            Publisher {
                shared: &self.shared,
            },
            Consumer {
                shared: &self.shared,
            },
        )
    }
}

impl<T> Default for Latest<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// 创建单槽所有者；调用 [`Latest::split`] 借用生产与消费端。
pub const fn latest<T>() -> Latest<T> {
    Latest::new()
}

/// 单生产端；关闭后消费者仍可取走最后一个待消费值。
pub struct Publisher<'a, T> {
    shared: &'a Shared<T>,
}

/// 单消费端；关闭时在锁外释放尚未消费的值。
pub struct Consumer<'a, T> {
    shared: &'a Shared<T>,
}

/// 消费端已关闭，发布失败；保留未交付值的所有权。
pub struct PublishError<T>(
    /// 未交付的完整值。
    pub T,
);

impl<T> fmt::Debug for PublishError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PublishError(consumer closed)")
    }
}

impl<T> fmt::Display for PublishError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("完整值交接的消费端已关闭")
    }
}

impl<T> std::error::Error for PublishError<T> {}

/// 等待取得完整值、期限到达或生产端关闭。
#[derive(Debug, Eq, PartialEq)]
pub enum Wait<T> {
    /// 移交一个完整值；该值此后由调用方独占。
    Value(T),
    /// 未取得值且等待期限已到。
    Timeout,
    /// 生产端已关闭，槽位也已排空。
    Closed,
}

impl<T> Publisher<'_, T> {
    /// 发布并覆盖未消费的旧值；消费端关闭时归还传入值。
    ///
    /// 覆盖值在解锁后析构；析构可能耗时或 panic，均不会中毒内部锁。
    /// 新值在析构旧值之前已经完整发布并唤醒消费者。
    pub fn publish(&mut self, value: T) -> Result<(), PublishError<T>> {
        let old = {
            let mut state = self.shared.lock();
            if !state.consumer_open {
                return Err(PublishError(value));
            }
            state.pending.replace(value)
        };
        self.shared.changed.notify_one();
        drop(old);
        Ok(())
    }
}

impl<T> Consumer<'_, T> {
    /// 不等待地取走当前完整值；空槽返回 `None`，不区分空槽与生产端关闭。
    pub fn take(&mut self) -> Option<T> {
        self.shared.lock().pending.take()
    }

    /// 等到完整值、生产端关闭或绝对平台期限；虚假唤醒会重新检查条件。
    ///
    /// 已有值优先于超时或关闭，生产端关闭后仍交付最后一个值。
    /// 不创建线程或轮询；等待释放锁。需要停止时可关闭生产端唤醒等待者。
    /// 此调用不保证线程恰在期限被调度运行；外部停止标志仍由应用检查。
    pub fn wait_until(&mut self, deadline: Instant) -> Wait<T> {
        let mut state = self.shared.lock();
        loop {
            if let Some(value) = state.pending.take() {
                return Wait::Value(value);
            }
            if !state.publisher_open {
                return Wait::Closed;
            }
            let now = Instant::now();
            if now >= deadline {
                return Wait::Timeout;
            }
            let (next, _) = self
                .shared
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
    }
}

impl<T> Drop for Publisher<'_, T> {
    fn drop(&mut self) {
        self.shared.lock().publisher_open = false;
        self.shared.changed.notify_one();
    }
}

impl<T> Drop for Consumer<'_, T> {
    fn drop(&mut self) {
        let pending = {
            let mut state = self.shared.lock();
            state.consumer_open = false;
            state.pending.take()
        };
        self.shared.changed.notify_one();
        drop(pending);
    }
}

#[cfg(test)]
mod tests;
