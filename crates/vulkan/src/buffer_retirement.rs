//! 大显存列退出资源图后的驱动释放。工作线程只收到独占 buffer，不访问队列或图。
use crate::{Device, buffer::Buffer};
use std::sync::{Mutex, atomic::Ordering, mpsc};
use std::thread;

const MIN_BYTES: u64 = 16 << 20;
const QUEUE_BYTES: u64 = 1 << 30;

struct Accounts {
    done: mpsc::Receiver<u64>,
    pending: u64,
}

pub(crate) struct BufferRetirement {
    jobs: Option<mpsc::SyncSender<Buffer>>,
    accounts: Mutex<Accounts>,
    worker: Option<thread::JoinHandle<()>>,
}
impl BufferRetirement {
    /// # Safety
    /// 唯一 Context 在销毁宿主 device 或 loader 前先 drop 本所有者。
    pub(crate) unsafe fn new(device: &Device) -> Result<Self, std::io::Error> {
        let raw = device.raw.clone();
        let (jobs, buffers) = mpsc::sync_channel::<Buffer>(4);
        let (ack, done) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("vulkan-buffer-retire".into())
            .spawn(move || {
                for mut buffer in buffers {
                    let bytes = buffer.allocation_bytes();
                    // SAFETY: 只有图已移除、GPU 使用已结束的独立分配可入队。宿主设备保活到 join。
                    unsafe { buffer.destroy_handles(&raw) };
                    let _ = ack.send(bytes);
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            accounts: Mutex::new(Accounts { done, pending: 0 }),
            worker: Some(worker),
        })
    }
    pub(crate) fn account(&self, device: &Device) {
        let mut state = self.accounts.lock().expect("回收计数临界区无用户回调");
        let freed: u64 = state.done.try_iter().sum();
        state.pending -= freed;
        device.buffer_bytes.fetch_sub(freed, Ordering::Relaxed);
    }
    /// 小列或额度已满时交还调用者立即释放。单个超额列可独占一个空队列。
    pub(crate) fn enqueue(&mut self, device: &Device, buffer: Buffer) -> Result<(), Buffer> {
        let n = buffer.allocation_bytes();
        if n < MIN_BYTES {
            return Err(buffer);
        }
        self.account(device);
        let state = self.accounts.get_mut().expect("独占回收所有者");
        if state.pending != 0 && n > QUEUE_BYTES.saturating_sub(state.pending) {
            return Err(buffer);
        }
        match self.jobs.as_ref().expect("活动回收线程").try_send(buffer) {
            Ok(()) => {
                state.pending += n;
                Ok(())
            }
            Err(mpsc::TrySendError::Full(b) | mpsc::TrySendError::Disconnected(b)) => Err(b),
        }
    }
    pub(crate) fn drain(&mut self, device: &Device) {
        let state = self.accounts.get_mut().expect("独占回收所有者");
        while state.pending != 0 {
            let freed = state.done.recv().expect("所有已接收的分配都会交还字节");
            state.pending -= freed;
            device.buffer_bytes.fetch_sub(freed, Ordering::Relaxed);
        }
    }
}
impl Drop for BufferRetirement {
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .expect("回收线程只调用无返回值的 Vulkan 析构和完成通知");
        }
    }
}
