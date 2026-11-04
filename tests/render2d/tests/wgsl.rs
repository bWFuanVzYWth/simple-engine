//! 生产 Web WGSL 经 wgpu 实际解析与验证。
#![forbid(unsafe_code)]

use simple_engine::{gpu_log, render2d::gpu, vulkan::interop::WgpuContext};

#[test]
#[ignore = "需要真实 Vulkan；初始化失败计为失败"]
fn production_wgsl_modules_pass_wgpu_validation() -> Result<(), Box<dyn std::error::Error>> {
    gpu_log::checked(|| {
        let context = pollster::block_on(WgpuContext::compute())?;
        let device = context.device();
        for (label, source) in [
            ("render2d vertex", gpu::VERTEX_WGSL),
            ("render2d fragment", gpu::FRAGMENT_WGSL),
        ] {
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            if let Some(error) = pollster::block_on(scope.pop()) {
                return Err(format!("生产 WGSL 模块 {label} 验证失败：{error}").into());
            }
        }
        Ok(())
    })
}
