//! 生产 shader 的执行与完整输入重放；每个样本输出一行 `uint4`。
//!
//! 回读先检查逐样本执行标识，防止未执行的零输出被误判为性质通过。性质 mask、统计量
//! 和 schema 匹配在此固定；GPU 环境按 compute/RT 能力复用，CPU 断言不占环境锁。

use simple_engine::{
    gpu_test::{self, Capability, Failure, Shader, with_shared},
    vulkan::interop::{RayShader, RayTracingPipeline},
    wgpu,
};

/// 测试环境失败或 fixture/schema 不匹配。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 设备、提交、回读或 validation 错误。
    #[error(transparent)]
    Gpu(#[from] gpu_test::Error),
    /// 未知入口、输入形状错误或 GPU 执行标识不完整。
    #[error("{0}")]
    Input(&'static str),
}

/// 一批可定位的原始输入与 GPU 结果，不在这里抹去浮点位模式。
pub struct Batch {
    /// 不含共享前缀的连续输入字。
    pub input: Vec<u32>,
    /// 每个样本的输入字数。
    pub stride: usize,
    /// 记录失败时添加的共享上下文；RT 为三角形数及紧密顶点，compute 为空。
    pub prefix: Vec<u32>,
    /// 每个样本的一行 GPU 输出，内容由入口 schema 定义。
    pub actual: Vec<[u32; 4]>,
    /// 实际执行设备的名称。
    pub device: String,
    /// 实际执行驱动的描述。
    pub driver: String,
    /// 实际使用的入口字节指纹；RT 同时包含三个阶段的入口名和字节。
    pub shader_blake3: String,
}

/// 按 little-endian 编码输入字，不作浮点数值转换。
pub fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

/// 选择已构建的 compute 入口；只接受此测试包声明的六个入口，工作组为 64×1×1。
pub fn shader(entry: &str) -> Result<Shader<'static>, Error> {
    let (spirv, entry) = match entry {
        "properties_main" => (
            include_bytes!(concat!(env!("OUT_DIR"), "/properties_main.spv")).as_slice(),
            "properties_main",
        ),
        "known_main" => (
            include_bytes!(concat!(env!("OUT_DIR"), "/known_main.spv")).as_slice(),
            "known_main",
        ),
        "surface_main" => (
            include_bytes!(concat!(env!("OUT_DIR"), "/surface_main.spv")).as_slice(),
            "surface_main",
        ),
        "connection_bound_main" => (
            include_bytes!(concat!(env!("OUT_DIR"), "/connection_bound_main.spv")).as_slice(),
            "connection_bound_main",
        ),
        "connection_degenerate_main" => (
            include_bytes!(concat!(env!("OUT_DIR"), "/connection_degenerate_main.spv")).as_slice(),
            "connection_degenerate_main",
        ),
        "connection_load_main" => (
            include_bytes!(concat!(env!("OUT_DIR"), "/connection_load_main.spv")).as_slice(),
            "connection_load_main",
        ),
        _ => return Err(Error::Input("未知 shader 入口")),
    };
    Ok(Shader {
        spirv,
        entry,
        workgroup: [64, 1, 1],
    })
}

fn decode(bytes: &[u8]) -> Vec<[u32; 4]> {
    bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|row| {
            std::array::from_fn(|i| u32::from_le_bytes(row[i * 4..i * 4 + 4].try_into().unwrap()))
        })
        .collect()
}

/// 按 stride 批量运行入口并完整回读；样本数须非零且与输入长度整除匹配。
/// 调用方保证 stride 与所选入口的 schema 一致，输出执行标识必须覆盖每个样本。
pub fn compute(input: Vec<u32>, stride: usize, entry: &str) -> Result<Batch, Error> {
    if stride == 0 || input.is_empty() || !input.len().is_multiple_of(stride) {
        return Err(Error::Input("compute 输入长度与 schema 不符"));
    }
    let shader = shader(entry)?;
    let count = input.len() / stride;
    let encoded = bytes(&input);
    let (output, info) = with_shared(Capability::Compute, |gpu| {
        let output = gpu.compute(
            shader,
            &encoded,
            count * 16,
            [(count as u32).div_ceil(64), 1, 1],
        )?;
        Ok((output, gpu.context().adapter_info()))
    })?;
    let actual = decode(&output);
    let marker = if matches!(
        entry,
        "connection_bound_main" | "connection_degenerate_main"
    ) {
        3
    } else {
        1
    };
    if actual.len() != count
        || actual
            .iter()
            .enumerate()
            .any(|(i, row)| row[marker] != i as u32 + 1)
    {
        return Err(Error::Input("GPU compute 执行标识不完整"));
    }
    Ok(Batch {
        input,
        stride,
        prefix: Vec::new(),
        actual,
        device: info.name,
        driver: info.driver_info,
        shader_blake3: blake3::hash(shader.spirv).to_hex().to_string(),
    })
}

/// 在 opaque 三角形场景中检验 outgoing 自交或连接遮挡，参数每样本四字。
/// 首个三角形还作为表面重建输入；返回前完成追踪、回读和执行标识校验。
pub fn rays(
    geometry: &[[[f32; 3]; 3]],
    parameters: Vec<u32>,
    outgoing: bool,
) -> Result<Batch, Error> {
    if geometry.is_empty() || parameters.is_empty() || !parameters.len().is_multiple_of(4) {
        return Err(Error::Input("光追输入为空或与 schema 不符"));
    }
    let mut prefix = vec![geometry.len() as u32];
    prefix.extend(
        geometry
            .iter()
            .flatten()
            .flatten()
            .map(|value| value.to_bits()),
    );
    let mut encoded: Vec<_> = geometry[0]
        .iter()
        .flat_map(|vertex| {
            vertex
                .iter()
                .copied()
                .chain([0.0])
                .flat_map(f32::to_le_bytes)
        })
        .collect();
    encoded.extend(bytes(&parameters));
    let count = parameters.len() / 4;
    let raygen = if outgoing {
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/outgoing_main.spv")),
            entry: c"outgoing_main",
        }
    } else {
        RayShader {
            spirv: include_bytes!(concat!(env!("OUT_DIR"), "/connection_main.spv")),
            entry: c"connection_main",
        }
    };
    let miss = RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/miss_main.spv")),
        entry: c"miss_main",
    };
    let hit = RayShader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/hit_main.spv")),
        entry: c"hit_main",
    };
    let mut shader_hash = blake3::Hasher::new();
    for shader in [&raygen, &miss, &hit] {
        shader_hash.update(shader.entry.to_bytes());
        shader_hash.update(shader.spirv);
    }
    let (output, info) = with_shared(Capability::RayTracing, |gpu| {
        let context = gpu.context_mut();
        let scene = context.acceleration_structure(geometry)?;
        let input = context.buffer("ray cases", &encoded, wgpu::BufferUsages::STORAGE)?;
        let output = context.buffer(
            "ray results",
            &vec![0; count * 16],
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        )?;
        let pipeline = RayTracingPipeline::new_buffers(
            context,
            scene,
            [raygen, miss, hit],
            &[input, output.clone()],
        )?;
        let command = pipeline.encode(context, [count as u32, 1, 1])?;
        context.submit([command])?;
        let info = context.adapter_info();
        let result = gpu.readback(output.buffer())?;
        // 此 helper 不发布 native 身份；既有 readback 已等待同队列完成。
        gpu.context_mut().native().collect(&[])?;
        Ok((result, info))
    })?;
    let actual = decode(&output);
    if actual.len() != count
        || actual
            .iter()
            .enumerate()
            .any(|(i, row)| row[1] != i as u32 + 1)
    {
        return Err(Error::Input("GPU 光追执行标识不完整"));
    }
    Ok(Batch {
        input: parameters,
        stride: 4,
        prefix,
        actual,
        device: info.name,
        driver: info.driver_info,
        shader_blake3: shader_hash.finalize().to_hex().to_string(),
    })
}

fn mask(case: &str) -> Option<u32> {
    Some(match case {
        "random_unit_interval" | "surface_position" | "mis_known_weights" => 1,
        "cosine_unit" | "surface_normal" | "lambertian_known_value" => 2,
        "cosine_hemisphere" | "spawn_sides" | "lambertian_known_pdf" => 4,
        "lambertian_throughput" => 8,
        "mis_complement" => 16,
        "outgoing_self_hit" | "connection_endpoint" | "near_occluder" => 3,
        _ => return None,
    })
}

/// 优先使用记录中的完整输入重新执行性质；不依赖当前随机生成器或默认批量。
///
/// 严格匹配 case/schema/长度，未知组合返回错误。成功返回的 JSON 同时携带当前与原
/// shader 指纹及 `passed`；`Ok` 仅表示执行完成，不表示原失败性质已经通过。
pub fn replay(failure: &Failure) -> Result<serde_json::Value, Error> {
    let input = failure.input.clone();
    let batch = match failure.schema.as_str() {
        "known-v1"
            if input.len() == 16
                && matches!(
                    failure.case.as_str(),
                    "mis_known_weights" | "lambertian_known_value" | "lambertian_known_pdf"
                ) =>
        {
            compute(input, 16, "known_main")?
        }
        "sampling-v1"
            if input.len() == 16
                && matches!(
                    failure.case.as_str(),
                    "random_unit_interval"
                        | "cosine_unit"
                        | "cosine_hemisphere"
                        | "lambertian_throughput"
                        | "mis_complement"
                ) =>
        {
            compute(input, 16, "properties_main")?
        }
        "surface-v1"
            if input.len() == 48
                && matches!(
                    failure.case.as_str(),
                    "surface_position" | "surface_normal" | "spawn_sides"
                ) =>
        {
            compute(input, 48, "surface_main")?
        }
        "sampling-moment-v1"
            if !input.is_empty()
                && input.len().is_multiple_of(16)
                && failure.case == "cosine_first_moment" =>
        {
            compute(input, 16, "properties_main")?
        }
        "connection-bound-v1" if input.len() == 40 && failure.case == "endpoint_bound" => {
            compute(input, 40, "connection_bound_main")?
        }
        "connection-degenerate-v1"
            if input.len() == 20 && failure.case == "connection_degenerate" =>
        {
            compute(input, 20, "connection_degenerate_main")?
        }
        "ray-geometry-v1"
            if matches!(
                failure.case.as_str(),
                "outgoing_self_hit" | "connection_endpoint" | "near_occluder"
            ) =>
        {
            let count = input.first().copied().unwrap_or(0) as usize;
            if !matches!(count, 1 | 2) || input.len() != 1 + count * 9 + 4 {
                return Err(Error::Input("光追重放输入长度无效"));
            }
            let geometry: Vec<_> = input[1..1 + count * 9]
                .as_chunks::<9>()
                .0
                .iter()
                .map(|triangle| {
                    std::array::from_fn(|i| {
                        std::array::from_fn(|j| f32::from_bits(triangle[i * 3 + j]))
                    })
                })
                .collect();
            rays(
                &geometry,
                input[1 + count * 9..].to_vec(),
                failure.case == "outgoing_self_hit",
            )?
        }
        _ => return Err(Error::Input("未知或不匹配的重放 case/schema/input")),
    };
    let passed = if let Some(mask) = mask(&failure.case) {
        batch.actual.iter().all(|row| row[0] & mask == 0)
    } else {
        match failure.case.as_str() {
            "endpoint_bound" => {
                let [base, near, far, _] = batch.actual[0].map(f32::from_bits);
                near > base && far > near
            }
            "connection_degenerate" => batch.actual[0][..2] == [0, 0],
            "cosine_first_moment" => {
                let mean = batch
                    .actual
                    .iter()
                    .map(|row| f64::from(f32::from_bits(row[2])))
                    .sum::<f64>()
                    / batch.actual.len() as f64;
                (mean - 2.0 / 3.0).abs() < 0.005
            }
            _ => return Err(Error::Input("未知重放性质")),
        }
    };
    Ok(
        serde_json::json!({ "case": failure.case, "schema": failure.schema, "seed": failure.seed,
        "stream": failure.stream, "index": failure.index, "passed": passed,
        "device": batch.device, "driver": batch.driver,
        "recorded_shader_blake3": failure.shader_blake3, "shader_blake3": batch.shader_blake3,
        "actual": batch.actual }),
    )
}
