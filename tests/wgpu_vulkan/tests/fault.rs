//! 提交验证失败后，native 所有者隔离不确定记录并保留分配。
#![forbid(unsafe_code)]

use simple_engine::{
    gpu_log,
    vulkan::{
        DeviceFailure, Error as VulkanError,
        interop::WgpuContext,
        native::{Buffer, Commands},
    },
    wgpu,
};
use std::error::Error;

#[test]
#[ignore = "需要真实 Vulkan；主任务统一调度"]
fn rejected_mixed_submit_quarantines_native_resources() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let buffer = Buffer::new(context.native(), 4, true)?;
        let allocated = context.allocated_native_bytes();
        assert!(allocated > 0);
        let mut commands = Commands::new(context.native())?;
        commands.clear_buffer(buffer)?;
        let recorded = commands.finish(context.native())?;
        let native = context.record_native(recorded)?;
        context.native().collect(&[])?;

        let destroyed = context.device().create_buffer(&wgpu::BufferDescriptor {
            label: Some("rejected submission buffer"),
            size: 4,
            usage: wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = context.device().create_command_encoder(&Default::default());
        encoder.clear_buffer(&destroyed, 0, None);
        let ordinary = encoder.finish();
        destroyed.destroy();

        assert!(matches!(
            context.submit([native, ordinary.into()]),
            Err(VulkanError::WgpuOperation(wgpu::Error::Validation { .. }))
        ));
        assert!(matches!(
            context.submit([]),
            Err(VulkanError::DeviceFault(DeviceFailure::Validation))
        ));
        assert!(matches!(
            context.native().collect(&[]),
            Err(VulkanError::DeviceFault(DeviceFailure::Validation))
        ));
        assert!(matches!(
            buffer.write(context.native(), 0, &[1, 2, 3, 4]),
            Err(VulkanError::DeviceFault(DeviceFailure::Validation))
        ));

        context.device().poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })?;
        context.completed_serial();
        assert_eq!(context.allocated_native_bytes(), allocated);
        Ok(())
    })
}
