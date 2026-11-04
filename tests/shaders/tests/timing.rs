//! 真实 GPU timestamp、计数器与异步槽生命周期回归。
use simple_engine::{
    gpu_test::{Capability, Gpu},
    vulkan::interop::{RayShader, RayTracingPipeline},
    wgpu,
};

#[test]
#[ignore = "需要 Vulkan KHR 光追及 timestamp"]
fn timed_native_interval_preserves_output() {
    let mut gpu = Gpu::new(Capability::RayTracing, 0, true).unwrap();
    let (control, timed) = {
        let context = gpu.context_mut();
        let geometry = [[
            [0.0, 0.0, 0.0],
            [2000.0, 1000.0, 0.0],
            [0.0, 1000.0, 2000.0],
        ]];
        let scene = context.acceleration_structure(&geometry).unwrap();
        let mut words = Vec::new();
        for vertex in geometry[0] {
            words.extend(vertex.into_iter().chain([0.0]).map(f32::to_bits));
        }
        for _ in 0..4096 {
            words.extend([0.2f32, 0.3, 20.0, 0.0].map(f32::to_bits));
        }
        let input = context
            .buffer(
                "timed ray input",
                &shader_tests::run::bytes(&words),
                wgpu::BufferUsages::STORAGE,
            )
            .unwrap();
        let output = context
            .buffer(
                "timed ray output",
                &vec![0; 4096 * 16],
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            )
            .unwrap();
        let pipeline = RayTracingPipeline::new_buffers(
            context,
            scene,
            [
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/connection_main.spv")),
                    entry: c"connection_main",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/miss_main.spv")),
                    entry: c"miss_main",
                },
                RayShader {
                    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/hit_main.spv")),
                    entry: c"hit_main",
                },
            ],
            &[input, output.clone()],
        )
        .unwrap();
        let command = pipeline.encode(context, [4096, 1, 1]).unwrap();
        context.submit([command]).unwrap();
        let control = gpu.readback(output.buffer()).unwrap();
        let command = pipeline.encode(gpu.context_mut(), [4096, 1, 1]).unwrap();
        let sample = gpu.measure(vec![command]).unwrap();
        eprintln!("native interval: {sample:?}");
        (control, gpu.readback(output.buffer()).unwrap())
    };
    gpu.finish().unwrap();
    assert_eq!(timed, control);
}
