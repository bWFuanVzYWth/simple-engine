// 仅注入只读源的临时副本；用原 WGSL 冻结独立 oracle，不参与生产构建。

fn reference_half(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = (bits >> 10) & 31;
    let mantissa = u32::from(bits & 1023);
    if exponent == 0 {
        let value = mantissa as f32 / 16_777_216.0;
        if sign == 0 { value } else { -value }
    } else if exponent == 31 {
        f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13))
    } else {
        f32::from_bits(sign | ((u32::from(exponent) + 112) << 23) | (mantissa << 13))
    }
}

fn reference_samples(
    d: &wgpu::Device,
    q: &wgpu::Queue,
    texture: &wgpu::Texture,
    bpp: u32,
) -> Result<serde_json::Value> {
    let bytes = read_texture(d, q, texture, bpp)?;
    let mut samples = Vec::new();
    let nx = texture.width().min(11);
    let ny = texture.height().min(9);
    for j in 0..ny {
        let y = if ny == 1 {
            0
        } else {
            j * (texture.height() - 1) / (ny - 1)
        };
        for i in 0..nx {
            let x = if nx == 1 {
                0
            } else {
                i * (texture.width() - 1) / (nx - 1)
            };
            let offset = ((y * texture.width() + x) * bpp) as usize;
            let value: [f32; 4] = std::array::from_fn(|lane| {
                if bpp == 8 {
                    reference_half(u16::from_le_bytes(
                        bytes[offset + 2 * lane..offset + 2 * lane + 2]
                            .try_into()
                            .unwrap(),
                    ))
                } else {
                    f32::from_le_bytes(
                        bytes[offset + 4 * lane..offset + 4 * lane + 4]
                            .try_into()
                            .unwrap(),
                    )
                }
            });
            if value.iter().any(|x| !x.is_finite()) {
                return Err("nonfinite reference field".into());
            }
            samples.push(serde_json::json!({"at":[x,y],"value":value}));
        }
    }
    Ok(serde_json::Value::Array(samples))
}

async fn reference(output: &std::path::Path) -> Result<()> {
    std::fs::create_dir_all(output)?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::VALIDATION,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        })
        .await?;
    let mut limits = wgpu::Limits::default();
    limits.max_storage_buffer_binding_size = adapter
        .limits()
        .max_storage_buffer_binding_size
        .min(2 * 1024 * 1024 * 1024 - 256);
    limits.max_buffer_size = adapter.limits().max_buffer_size.min(2 * 1024 * 1024 * 1024);
    limits.max_texture_dimension_2d = 8192;
    let (d, q) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("independent sky_tracer WGSL oracle"),
            required_limits: limits,
            required_features: adapter.features() & wgpu::Features::TIMESTAMP_QUERY,
            ..Default::default()
        })
        .await?;
    eprintln!("reference adapter: {:?}", adapter.get_info());
    let mut config = Config::balanced();
    config.skip_unused_ground = false;
    let mut renderer = Renderer::new(
        &d,
        &Model::earth()?,
        &Wavelengths::optimized_four(),
        0.18,
        config,
    )?;
    let solve = renderer.rebuild(&d, &q)?;
    eprintln!("source solve {:.3}s", solve.wall_seconds);
    std::fs::write(
        output.join("solve.json"),
        serde_json::to_vec_pretty(&solve)?,
    )?;
    let field = renderer.field.as_ref().ok_or("source field")?;
    let fields = serde_json::json!({
        "tau":reference_samples(&d,&q,&renderer._tau,8)?,
        "source":reference_samples(&d,&q,&field.source,8)?,
        "mean":reference_samples(&d,&q,&field._mean,16)?,
        "high":reference_samples(&d,&q,&field._high,16)?,
        "ground":reference_samples(&d,&q,&field._ground,16)?
    });
    std::fs::write(
        output.join("source.rgba16f"),
        renderer.export_source(&d, &q)?,
    )?;
    std::fs::write(
        output.join("optical.rgba16f"),
        renderer.export_optical(&d, &q)?,
    )?;
    let mut skies = Vec::new();
    for (height, elevation) in [(0.2_f32, 35.0_f32), (0.2, -6.0), (40.0, 5.0), (400.0, 0.0)] {
        let view = View {
            yaw_deg: 0.0,
            pitch_deg: 0.0,
            fov_y_deg: 80.0,
            sun_azimuth_deg: 0.0,
            sun_elevation_deg: elevation,
            altitude_km: height,
        };
        let mut encoder = d.create_command_encoder(&Default::default());
        renderer.render(&q, &mut encoder, view);
        q.submit([encoder.finish()]);
        d.poll(wgpu::PollType::wait_indefinitely())?;
        skies.push(
            serde_json::json!({"altitude_km":height,"sun_elevation":elevation.to_radians(),
            "samples":reference_samples(&d,&q,&renderer._sky,16)?}),
        );
    }
    let queries: Vec<[f32; 4]> = [
        ([0.0, 1.0, 0.0], 0.0, [0.0, 0.6, 0.8], 0.2),
        ([0.0, 0.0, 1.0], 1.0, [0.0, 0.6, 0.8], 0.2),
        ([0.0, 0.0, 1.0], 8.0, [0.0, 0.6, 0.8], 0.2),
        ([0.0, 0.0, 1.0], 80.0, [0.0, 0.6, 0.8], 0.2),
        ([0.6, 0.8, 0.0], 2.0, [0.0, 0.6, 0.8], 0.001),
        ([0.0, -1.0, 0.0], 0.0, [0.0, 0.6, 0.8], 0.2),
        ([0.0, 1.0, 0.0], 3.0, [0.0, 0.6, 0.8], 400.0),
        ([0.0, -1.0, 0.0], 3.0, [0.0, 0.6, 0.8], 400.0),
        ([0.0, -1.0, 0.0], 380.0, [0.0, 0.6, 0.8], 400.0),
        ([0.0, 1.0, 0.0], 5.0, [0.0, 0.0, 1.0], 40.0),
    ]
    .into_iter()
    .flat_map(|(ray, segment, sun, h)| {
        [
            [ray[0], ray[1], ray[2], segment],
            [sun[0], sun[1], sun[2], h],
        ]
    })
    .collect();
    let source = shader_source()
        + r#"
@group(0) @binding(24) var<storage,read> probe_queries:array<vec4<f32>>;
@group(0) @binding(25) var<storage,read_write> probe_results:array<vec4<f32>>;
@compute @workgroup_size(64) fn reference_probe(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&probe_queries)/2u{return;}
    let ray=probe_queries[2u*id.x];let sun=probe_queries[2u*id.x+1u];
    let result=integrate_at(ray.xyz,sun.xyz,sun.w,ray.w,p.size_steps.z);
    probe_results[2u*id.x]=result.light;probe_results[2u*id.x+1u]=result.transmittance;
}
"#;
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("source transport probe"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let constants = [
        ("SPECIALIZE", 1.0),
        ("FIXED_MAPPING_FLAGS", 63.0),
        (
            "FIXED_PHASE_WEIGHT",
            f64::from(renderer.medium.params.angular_mapping[0]),
        ),
        (
            "FIXED_CONE_WARP",
            f64::from(renderer.medium.params.angular_mapping[1]),
        ),
        ("SKIP_INACTIVE_SPECIES", 1.0),
        ("CACHE_PHASE_COORDINATE", 0.0),
    ];
    let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("source transport probe"),
        layout: None,
        module: &module,
        entry_point: Some("reference_probe"),
        compilation_options: wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        },
        cache: None,
    });
    let input = buffer(&d, "reference queries", bytemuck::cast_slice(&queries));
    let result = d.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reference result"),
        size: (queries.len() * 16) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let params = uniform(&d);
    q.write_buffer(&params, 0, bytemuck::bytes_of(&renderer.medium.params));
    let field = renderer.field.as_ref().ok_or("source field")?;
    let bindings = group(
        &d,
        &pipeline,
        &[
            (0, params.as_entire_binding()),
            (1, renderer.data.as_entire_binding()),
            (2, tv(&renderer.tau_view)),
            (3, tv(&field.source_view)),
            (4, tv(&field.mean_view)),
            (5, tv(&field.ground_view)),
            (6, wgpu::BindingResource::Sampler(&renderer.sampler)),
            (18, tv(&field.high_view)),
            (24, input.as_entire_binding()),
            (25, result.as_entire_binding()),
        ],
    );
    let mut encoder = d.create_command_encoder(&Default::default());
    dispatch(&mut encoder, &pipeline, &bindings, [1, 1], None);
    q.submit([encoder.finish()]);
    let result = read_buffer(&d, &q, &result, (queries.len() * 16) as u64)?;
    let mut transport = Vec::new();
    for (i, query) in queries.chunks_exact(2).enumerate() {
        let row: [f32; 8] = std::array::from_fn(|j| {
            f32::from_le_bytes(
                result[i * 32 + j * 4..i * 32 + j * 4 + 4]
                    .try_into()
                    .unwrap(),
            )
        });
        transport.push(serde_json::json!({"ray":query[0],"sun":query[1],"light":&row[..4],"transmittance":&row[4..]}));
    }
    let info = adapter.get_info();
    let document = serde_json::json!({"source_commit":"9e9430d52c50aa3df97fdf0fb6515abc67f44bef",
        "adapter":info.name,"backend":format!("{:?}",info.backend),"driver":info.driver,"config":renderer.config,
        "fields":fields,"skies":skies,"transport":transport});
    std::fs::write(
        output.join("reference.json"),
        serde_json::to_vec_pretty(&document)?,
    )?;
    eprintln!("wrote {}", output.join("reference.json").display());
    Ok(())
}
