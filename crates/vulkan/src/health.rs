//! 设备故障的唯一宿主所有者；回调只交付事件，不共享资源表。

use crate::{DeviceFailure, Error, submission::DeviceState};
use std::sync::{Mutex, mpsc};

pub(crate) struct Health {
    inner: Mutex<Inner>,
    lost_tx: mpsc::Sender<(wgpu::DeviceLostReason, String)>,
}

struct Inner {
    state: DeviceState,
    lost_rx: mpsc::Receiver<(wgpu::DeviceLostReason, String)>,
    pending: Option<Error>,
}

impl Inner {
    fn receive(&mut self) {
        while let Ok((reason, message)) = self.lost_rx.try_recv() {
            self.state.fail(DeviceFailure::Lost);
            if self.pending.is_none() {
                self.pending = Some(Error::DeviceLost { reason, message });
            }
        }
    }
}

impl Health {
    pub(crate) fn new() -> Self {
        let (lost_tx, lost_rx) = mpsc::channel();
        Self {
            inner: Mutex::new(Inner {
                state: DeviceState::new(),
                lost_rx,
                pending: None,
            }),
            lost_tx,
        }
    }

    pub(crate) fn attach(&self, device: &wgpu::Device) {
        self.observe(device, |_, _| {});
    }

    pub(crate) fn observe(
        &self,
        device: &wgpu::Device,
        observer: impl Fn(wgpu::DeviceLostReason, String) + Send + 'static,
    ) {
        let sender = self.lost_tx.clone();
        device.set_device_lost_callback(move |reason, message| {
            let _ = sender.send((reason, message.clone()));
            observer(reason, message);
        });
    }

    pub(crate) fn check(&self) -> Result<(), Error> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        inner.receive();
        if let Some(error) = inner.pending.take() {
            return Err(error);
        }
        match inner.state.failure() {
            Some(kind) => Err(Error::DeviceFault(kind)),
            None => Ok(()),
        }
    }

    pub(crate) fn fail(&self, kind: DeviceFailure) {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .state
            .fail(kind);
    }

    pub(crate) fn is_active(&self) -> bool {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        inner.receive();
        inner.state.failure().is_none()
    }
}

/// 只等待错误作用域结果，不等待 GPU。固定 native wgpu 后端的 pop 返回 ready future。
/// 三种错误均显式收取；同线程上调用方自己的外层 scope 不会截走本 owner 的错误。
pub(crate) fn checked<T>(
    device: &wgpu::Device,
    health: &Health,
    f: impl FnOnce() -> Result<T, Error>,
) -> Result<T, Error> {
    health.check()?;
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let value = f();
    let mut failure = None;
    for scope in [internal, memory, validation] {
        if let Some(error) = pollster::block_on(scope.pop())
            && failure.is_none()
        {
            failure = Some(error);
        }
    }
    if let Some(error) = failure {
        health.fail(match &error {
            wgpu::Error::Validation { .. } => DeviceFailure::Validation,
            wgpu::Error::OutOfMemory { .. } => DeviceFailure::OutOfMemory,
            wgpu::Error::Internal { .. } => DeviceFailure::Internal,
        });
        return Err(Error::WgpuOperation(error));
    }
    health.check()?;
    value
}

#[cfg(test)]
mod tests {
    use super::Health;
    use crate::{DeviceFailure, Error};

    #[test]
    fn observing_loss_keeps_original_diagnostic() {
        let health = Health::new();
        health
            .lost_tx
            .send((wgpu::DeviceLostReason::Unknown, "test loss".into()))
            .unwrap();
        assert!(!health.is_active());
        assert!(
            matches!(health.check(), Err(Error::DeviceLost { message, .. }) if message == "test loss")
        );
        assert!(matches!(
            health.check(),
            Err(Error::DeviceFault(DeviceFailure::Lost))
        ));
    }
}
