//! CPU 生产者与单个发布者之间的所有权交接。每个方向只容纳一批，不持有设备或世界借用。
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::thread;

/// 工作线程异常退出，或创建工作线程失败。
#[derive(Debug, thiserror::Error)]
pub enum ProducerError {
    /// 系统无法创建后台线程。
    #[error(transparent)]
    Spawn(#[from] std::io::Error),
    /// 前一任务尚未领取。
    #[error("地形生产者尚有在途任务")]
    Busy,
    /// 线程结束，正在处理的数据不再可用。
    #[error("地形 CPU 生产线程已经退出")]
    Stopped,
}

/// 一个在途 CPU 批次。调用者移交独占输入，后台可使用自己持有的线程池分块计算。
/// 绘制调用用 poll 领取结果；wait 供显式同步入口和资源世代切换使用。
pub struct Producer<T> {
    jobs: Option<SyncSender<Box<dyn FnOnce() -> T + Send>>>,
    results: Receiver<T>,
    thread: Option<thread::JoinHandle<()>>,
    busy: bool,
}
impl<T: Send + 'static> Producer<T> {
    /// 创建常驻任务线程。内核错误属于 T，线程创建错误保留系统来源。
    pub fn new(name: &str) -> Result<Self, ProducerError> {
        let (jobs, incoming) = mpsc::sync_channel::<Box<dyn FnOnce() -> T + Send>>(1);
        let (out, results) = mpsc::sync_channel(1);
        let thread = thread::Builder::new().name(name.into()).spawn(move || {
            while let Ok(job) = incoming.recv() {
                if out.send(job()).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self {
            jobs: Some(jobs),
            results,
            thread: Some(thread),
            busy: false,
        })
    }
    /// 尚有已派发、未领取的批次。
    pub const fn busy(&self) -> bool {
        self.busy
    }
    /// 向空闲生产者移交任务，不等待任务执行或之前的消费者。重复派发返回 Busy。
    pub fn submit<F: FnOnce() -> T + Send + 'static>(
        &mut self,
        job: F,
    ) -> Result<(), ProducerError> {
        let sender = self.jobs.as_ref().ok_or(ProducerError::Stopped)?;
        if self.busy {
            return Err(ProducerError::Busy);
        }
        match sender.try_send(Box::new(job)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => unreachable!("空闲生产者已经交还上一任务"),
            Err(mpsc::TrySendError::Disconnected(_)) => return Err(self.stop()),
        }
        self.busy = true;
        Ok(())
    }
    /// 非阻塞领取唯一结果；None 表示尚未完成或当前空闲。
    pub fn poll(&mut self) -> Result<Option<T>, ProducerError> {
        if self.jobs.is_none() {
            return Err(ProducerError::Stopped);
        }
        if !self.busy {
            return Ok(None);
        }
        match self.results.try_recv() {
            Ok(value) => {
                self.busy = false;
                Ok(Some(value))
            }
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(self.stop()),
        }
    }
    /// 显式等待并领取在途结果；空闲时返回 None。
    pub fn wait(&mut self) -> Result<Option<T>, ProducerError> {
        if self.jobs.is_none() {
            return Err(ProducerError::Stopped);
        }
        if !self.busy {
            return Ok(None);
        }
        let result = self.results.recv().map_err(|_| self.stop())?;
        self.busy = false;
        Ok(Some(result))
    }
    fn stop(&mut self) -> ProducerError {
        self.jobs = None;
        self.busy = false;
        ProducerError::Stopped
    }
}
impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
