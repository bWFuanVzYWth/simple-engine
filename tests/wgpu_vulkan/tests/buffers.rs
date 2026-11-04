//! 队列初始化与后续局部写入的共享 buffer 契约。
#![forbid(unsafe_code)]

use simple_engine::{gpu_log, vulkan::interop::WgpuContext, wgpu};
use std::{error::Error, sync::mpsc};

#[test]
#[ignore = "需要 Vulkan 设备；由 GPU 验收串行运行"]
fn zeroed_storage_preserves_later_partial_writes() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let context = pollster::block_on(WgpuContext::compute())?;
        let storage = context.zeroed_buffer(
            "zeroed storage",
            256,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        )?;
        let value = 0xa91f_5b27_u32.to_ne_bytes();
        context.queue().write_buffer(storage.buffer(), 124, &value);
        let readback = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("zeroed readback"),
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = context.device().create_command_encoder(&Default::default());
        encoder.copy_buffer_to_buffer(storage.buffer(), 0, &readback, 0, 256);
        drop(storage);
        let submission = context.queue().submit([encoder.finish()]);
        let (send, receive) = mpsc::sync_channel(1);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        receive.recv()??;
        let mapped = readback.slice(..).get_mapped_range()?;
        let mut expected = [0; 256];
        expected[124..128].copy_from_slice(&value);
        assert_eq!(&*mapped, &expected);
        drop(mapped);
        readback.unmap();
        Ok(())
    })
}
