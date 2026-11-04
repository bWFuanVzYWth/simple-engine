//! 生产 Slang 2D 像素、裁剪与资源租约的真实 GPU 回归。
#![forbid(unsafe_code)]

use simple_engine::{
    gpu_test::{self, Capability, Gpu, Trial},
    render2d::{Canvas, Color, DrawList, Point, Rect, gpu::*, text::*},
    vulkan::interop::WgpuContext,
    wgpu,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const EXTENT: [u32; 2] = [64, 32];
const CONFIG: TargetConfig = TargetConfig {
    format: wgpu::TextureFormat::Rgba8Unorm,
    samples: 1,
};

fn renderer(context: &WgpuContext) -> Result<Renderer2d> {
    renderer_with_budgets(context, Budgets::default())
}

fn renderer_with_budgets(context: &WgpuContext, budgets: Budgets) -> Result<Renderer2d> {
    Ok(Renderer2d::new(
        context.device(),
        context.queue(),
        Shaders2d {
            vertex: context.shader(VERTEX_SPIRV, "vertex_main", [0; 3])?,
            fragment: context.shader(FRAGMENT_SPIRV, "fragment_main", [0; 3])?,
        },
        budgets,
    )?)
}

fn target(context: &WgpuContext) -> wgpu::Texture {
    context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("2D 回归目标"),
        size: wgpu::Extent3d {
            width: EXTENT[0],
            height: EXTENT[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: CONFIG.format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn attachment(view: &wgpu::TextureView) -> RenderTarget<'_> {
    RenderTarget {
        view,
        resolve: None,
        extent: EXTENT,
        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
        timestamp_writes: None,
    }
}

fn view() -> ViewParams {
    ViewParams::new(
        Rect::new(0.0, 0.0, EXTENT[0] as f32, EXTENT[1] as f32),
        EXTENT,
    )
}

fn read(gpu: &mut Gpu, texture: &wgpu::Texture) -> Result<Vec<u8>> {
    let context = gpu.context();
    let buffer = context.device().create_buffer(&wgpu::BufferDescriptor {
        label: Some("2D 像素回读"),
        size: u64::from(EXTENT[0] * EXTENT[1] * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let mut encoder = context.device().create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(EXTENT[0] * 4),
                rows_per_image: Some(EXTENT[1]),
            },
        },
        texture.size(),
    );
    context.queue().submit([encoder.finish()]);
    Ok(gpu.readback(&buffer)?)
}

fn pixel(bytes: &[u8], x: usize, y: usize) -> [u8; 4] {
    let at = (y * EXTENT[0] as usize + x) * 4;
    bytes[at..at + 4].try_into().unwrap()
}

fn render(gpu: &mut Gpu, scene: &DrawList, text: Option<&mut TextSystem>) -> Result<Vec<u8>> {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let target = target(context);
    let output = target.create_view(&Default::default());
    let prepared = match text {
        Some(text) => renderer.prepare_text(scene, text)?,
        None => renderer.prepare(scene)?,
    };
    let view = renderer.prepare_view(&prepared, view(), CONFIG)?;
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&view, attachment(&output))?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    read(gpu, &target)
}

fn transparent_order(gpu: &mut Gpu) -> Result {
    let mut scene = DrawList::new();
    let mut canvas = Canvas::new(&mut scene);
    let rect = Rect::new(0.0, 0.0, 64.0, 32.0);
    canvas.fill_rect(rect, Color::linear(1.0, 0.0, 0.0, 0.5));
    canvas.fill_rect(rect, Color::linear(0.0, 0.0, 1.0, 0.5));
    let color = pixel(&render(gpu, &scene, None)?, 20, 12);
    for (actual, expected) in color.into_iter().zip([64u8, 0, 128, 191]) {
        assert!(actual.abs_diff(expected) <= 1, "{color:?}");
    }
    Ok(())
}

fn batched_colors_match_linear_coverage(gpu: &mut Gpu) -> Result {
    let mut rng = gpu_test::PropertyRng::new(gpu_test::DEFAULT_SEED, 0x2d01);
    let mut scene = DrawList::new();
    let mut canvas = Canvas::new(&mut scene);
    let expected: Vec<_> = (0..256)
        .map(|index| {
            let words = rng.words(index);
            let [r, g, b, a] = std::array::from_fn(|i| gpu_test::unit_f32(words[i]));
            let color = Color::linear(r, g, b, a);
            canvas.fill_rect(
                Rect::new((index % 16 * 4) as f32, (index / 16 * 2) as f32, 4.0, 2.0),
                color,
            );
            color.0.map(|value| (value * 255.0).round() as u8)
        })
        .collect();
    let pixels = render(gpu, &scene, None)?;
    for (index, expected) in expected.into_iter().enumerate() {
        let actual = pixel(&pixels, index % 16 * 4 + 1, index / 16 * 2);
        assert!(
            actual
                .into_iter()
                .zip(expected)
                .all(|(a, b)| a.abs_diff(b) <= 1),
            "sample {index}: {actual:?} != {expected:?}"
        );
    }
    Ok(())
}

fn image(
    context: &WgpuContext,
    renderer: &mut Renderer2d,
    rgba: [u8; 4],
) -> Result<simple_engine::render2d::ImageKey> {
    let texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("2D 图片输入"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    context.queue().write_texture(
        texture.as_image_copy(),
        &rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        texture.size(),
    );
    Ok(renderer.register_image(SampledImage {
        view: texture.create_view(&Default::default()),
        sampler: context.device().create_sampler(&Default::default()),
    })?)
}

fn image_uses_premultiplied_alpha(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let key = image(context, &mut renderer, [128, 0, 0, 128])?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).image(
        key,
        Rect::new(0.0, 0.0, 64.0, 32.0),
        Rect::new(0.0, 0.0, 1.0, 1.0),
        Color::WHITE,
    );
    let prepared = renderer.prepare(&scene)?;
    let draw = renderer.prepare_view(&prepared, view(), CONFIG)?;
    let output = target(context);
    let output_view = output.create_view(&Default::default());
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&draw, attachment(&output_view))?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    assert_eq!(pixel(&read(gpu, &output)?, 20, 12), [128, 0, 0, 128]);
    Ok(())
}

fn image_keeps_its_painter_position(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let key = image(context, &mut renderer, [0, 128, 0, 128])?;
    let mut scene = DrawList::new();
    let mut canvas = Canvas::new(&mut scene);
    let bounds = Rect::new(0.0, 0.0, 64.0, 32.0);
    canvas.fill_rect(bounds, Color::linear(1.0, 0.0, 0.0, 1.0));
    canvas.image(key, bounds, Rect::new(0.0, 0.0, 1.0, 1.0), Color::WHITE);
    canvas.fill_rect(bounds, Color::linear(0.0, 0.0, 1.0, 0.5));
    let prepared = renderer.prepare(&scene)?;
    let draw = renderer.prepare_view(&prepared, view(), CONFIG)?;
    let output = target(context);
    let output_view = output.create_view(&Default::default());
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&draw, attachment(&output_view))?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    let color = pixel(&read(gpu, &output)?, 20, 12);
    for (actual, expected) in color.into_iter().zip([64u8, 64, 128, 255]) {
        assert!(actual.abs_diff(expected) <= 1, "{color:?}");
    }
    Ok(())
}

fn removed_image_key_is_rejected(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let key = image(context, &mut renderer, [255; 4])?;
    renderer.remove_image(key)?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).image(
        key,
        Rect::new(0.0, 0.0, 64.0, 32.0),
        Rect::new(0.0, 0.0, 1.0, 1.0),
        Color::WHITE,
    );
    assert!(renderer.prepare(&scene).is_err());
    Ok(())
}

fn clip_excludes_external_pixels(gpu: &mut Gpu) -> Result {
    let mut scene = DrawList::new();
    Canvas::new(&mut scene)
        .clipped(Rect::new(8.0, 8.0, 16.0, 16.0))
        .fill_rect(Rect::new(0.0, 0.0, 64.0, 32.0), Color::WHITE);
    let pixels = render(gpu, &scene, None)?;
    assert_eq!(pixel(&pixels, 7, 12), [0; 4]);
    assert_eq!(pixel(&pixels, 12, 12), [255; 4]);
    Ok(())
}

fn multiview_keeps_independent_uniforms(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 16.0, 32.0), Color::WHITE);
    let prepared = renderer.prepare(&scene)?;
    let first = renderer.prepare_view(&prepared, view(), CONFIG)?;
    let second = renderer.prepare_view(
        &prepared,
        ViewParams::new(Rect::new(0.0, 0.0, 32.0, 32.0), EXTENT),
        CONFIG,
    )?;
    let a = target(context);
    let b = target(context);
    let av = a.create_view(&Default::default());
    let bv = b.create_view(&Default::default());
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&first, attachment(&av))?;
    recording.encode(&second, attachment(&bv))?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    let a = read(gpu, &a)?;
    let b = read(gpu, &b)?;
    assert_eq!(pixel(&a, 20, 12), [0; 4]);
    assert_eq!(pixel(&b, 20, 12), [255; 4]);
    Ok(())
}

fn rounded_rect_excludes_corners(gpu: &mut Gpu) -> Result {
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).rounded_rect(Rect::new(4.0, 4.0, 24.0, 24.0), 8.0, Color::WHITE);
    let pixels = render(gpu, &scene, None)?;
    assert_eq!(pixel(&pixels, 4, 4), [0; 4]);
    assert_eq!(pixel(&pixels, 16, 16), [255; 4]);
    Ok(())
}

fn border_leaves_interior_empty(gpu: &mut Gpu) -> Result {
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).border(Rect::new(4.0, 4.0, 24.0, 24.0), 0.0, 3.0, Color::WHITE);
    let pixels = render(gpu, &scene, None)?;
    assert_eq!(pixel(&pixels, 5, 12), [255; 4]);
    assert_eq!(pixel(&pixels, 16, 16), [0; 4]);
    Ok(())
}

fn recorded_commands_keep_geometry_alive(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let a = target(context);
    let b = target(context);
    let mut commands = Vec::new();
    for (target, color) in [
        (&a, Color::linear(1.0, 0.0, 0.0, 1.0)),
        (&b, Color::linear(0.0, 0.0, 1.0, 1.0)),
    ] {
        let mut scene = DrawList::new();
        Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 64.0, 32.0), color);
        let prepared = renderer.prepare(&scene)?;
        let view = renderer.prepare_view(&prepared, view(), CONFIG)?;
        let output = target.create_view(&Default::default());
        let encoder = context.device().create_command_encoder(&Default::default());
        let mut recording = renderer.recording(encoder)?;
        recording.encode(&view, attachment(&output))?;
        commands.push(recording.finish()?);
        // 局部 Prepared 在提交前释放，唯一批次使 owner 保留对应几何版本。
    }
    for command in commands.into_iter().rev() {
        renderer.submit(command)?;
    }
    let a = read(gpu, &a)?;
    let b = read(gpu, &b)?;
    assert_eq!(pixel(&a, 20, 12), [255, 0, 0, 255]);
    assert_eq!(pixel(&b, 20, 12), [0, 0, 255, 255]);
    Ok(())
}

fn cancelled_upload_can_be_retried(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 64.0, 32.0), Color::WHITE);
    let package = renderer.prepare(&scene)?;
    let view = renderer.prepare_view(&package, view(), CONFIG)?;
    let output = target(context);
    let target = output.create_view(&Default::default());
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&view, attachment(&target))?;
    drop(recording);
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&view, attachment(&target))?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    assert_eq!(pixel(&read(gpu, &output)?, 20, 12), [255; 4]);
    Ok(())
}

fn prepared_view_reuses_geometry_across_completed_batches(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 64.0, 32.0), Color::WHITE);
    let package = renderer.prepare(&scene)?;
    let view = renderer.prepare_view(&package, view(), CONFIG)?;
    let output = target(context);
    let target = output.create_view(&Default::default());
    for _ in 0..2 {
        let encoder = context.device().create_command_encoder(&Default::default());
        let mut recording = renderer.recording(encoder)?;
        recording.encode(&view, attachment(&target))?;
        let recorded = recording.finish()?;
        renderer.submit(recorded)?;
        context.device().poll(wgpu::PollType::wait_indefinitely())?;
        renderer.collect();
    }
    assert_eq!(pixel(&read(gpu, &output)?, 20, 12), [255; 4]);
    Ok(())
}

fn recorded_atlas_copy_keeps_its_page_until_cancel(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer_with_budgets(
        context,
        Budgets {
            atlas_side: 32,
            max_atlas_pages: 1,
            ..Default::default()
        },
    )?;
    let mut text = TextSystem::new(
        [include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf").to_vec()],
        TextBudgets::default(),
    )?;
    let style = TextStyle {
        font_size: 24.0,
        line_height: 28.0,
        ..Default::default()
    };
    let first = text.layout("国", 64.0, style)?;
    let second = text.layout("田", 64.0, style)?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).text(
        &text.resolve(&first).unwrap(),
        Point::default(),
        Color::WHITE,
    );
    let package = renderer.prepare_text(&scene, &mut text)?;
    let empty = renderer.prepare(&DrawList::new())?;
    let empty = renderer.prepare_view(&empty, view(), CONFIG)?;
    let output = target(context).create_view(&Default::default());
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&empty, attachment(&output))?;
    let recorded = recording.finish()?;
    drop(package);
    scene.clear();
    Canvas::new(&mut scene).text(
        &text.resolve(&second).unwrap(),
        Point::default(),
        Color::WHITE,
    );
    assert!(matches!(
        renderer.prepare_text(&scene, &mut text),
        Err(RenderError::Busy)
    ));
    renderer.cancel(recorded)?;
    assert!(renderer.prepare_text(&scene, &mut text).is_ok());
    Ok(())
}

fn glyph_masks_preserve_premultiplied_coverage(gpu: &mut Gpu) -> Result {
    let mut text = TextSystem::new(
        [include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf").to_vec()],
        TextBudgets::default(),
    )?;
    let layout = text.layout("输入Aa", 60.0, TextStyle::default())?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).text(
        &text.resolve(&layout).unwrap(),
        Point::new(1.0, 1.0),
        Color::linear(1.0, 0.0, 0.0, 0.75),
    );
    let pixels = render(gpu, &scene, Some(&mut text))?;
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0 && p[3] < 190)
    );
    assert!(
        pixels
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] <= 192 && p[0].abs_diff(p[3]) <= 1 && p[1] == 0 && p[2] == 0)
    );
    Ok(())
}

fn glyph_keys_require_their_font_system(gpu: &mut Gpu) -> Result {
    let font = include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf");
    let mut original = TextSystem::new([font.to_vec()], TextBudgets::default())?;
    let mut other = TextSystem::new([font.to_vec()], TextBudgets::default())?;
    let layout = original.layout("字", 60.0, TextStyle::default())?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).text(
        &original.resolve(&layout).unwrap(),
        Point::default(),
        Color::WHITE,
    );
    let mut renderer = renderer(gpu.context())?;
    assert!(renderer.prepare_text(&scene, &mut other).is_err());
    Ok(())
}

fn glyphs_require_the_text_entry(gpu: &mut Gpu) -> Result {
    let font = include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf");
    let mut text = TextSystem::new([font.to_vec()], TextBudgets::default())?;
    let layout = text.layout("字", 60.0, TextStyle::default())?;
    let mut scene = DrawList::new();
    Canvas::new(&mut scene).text(
        &text.resolve(&layout).unwrap(),
        Point::default(),
        Color::WHITE,
    );
    let mut renderer = renderer(gpu.context())?;
    assert!(matches!(
        renderer.prepare(&scene),
        Err(RenderError::FontSystemMismatch)
    ));
    Ok(())
}

fn clipped_long_text_keeps_a_visible_glyph_workset(gpu: &mut Gpu) -> Result {
    let mut text = TextSystem::new(
        [include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf").to_vec()],
        TextBudgets {
            max_layout_bytes: 32 * 1024 * 1024,
            ..Default::default()
        },
    )?;
    let body: String = (0..21_845)
        .map(|i| char::from_u32(0x4e00 + i % 20_992).unwrap())
        .collect();
    let layout = text.layout(
        &body,
        60.0,
        TextStyle {
            wrap: false,
            hinting: false,
            ..Default::default()
        },
    )?;
    let caret = text
        .resolve(&layout)
        .unwrap()
        .caret_rect(TextCursor::new(body.len()))
        .unwrap();
    eprintln!("long text CPU {:?}", text.stats());
    let mut scene = DrawList::new();
    scene.canvas(Rect::new(0.0, 0.0, 64.0, 32.0)).text(
        &text.resolve(&layout).unwrap(),
        Point::new(60.0 - caret.x, 0.0),
        Color::WHITE,
    );
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let prepared = renderer.prepare_text(&scene, &mut text)?;
    let prepared = renderer.prepare_view(&prepared, view(), CONFIG)?;
    let target = target(context);
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(
        &prepared,
        attachment(&target.create_view(&Default::default())),
    )?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    let pixels = read(gpu, &target)?;
    let stats = renderer.stats();
    eprintln!(
        "{} bytes, {} glyph requests, {} atlas glyphs, {} atlas pages, {} atlas bytes",
        body.len(),
        text.resolve(&layout).unwrap().glyphs().len(),
        stats.atlas_glyphs,
        stats.atlas_pages,
        stats.atlas_bytes
    );
    assert!(stats.atlas_glyphs < 64 && stats.atlas_pages <= 1);
    assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0));
    Ok(())
}

fn growing_geometry_reuses_upload_capacity(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer(context)?;
    let target = target(context).create_view(&Default::default());
    let mut scene = DrawList::new();
    let mut retained = 0;
    for count in 1..=64 {
        Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), Color::WHITE);
        let prepared = renderer.prepare(&scene)?;
        let prepared = renderer.prepare_view(&prepared, view(), CONFIG)?;
        let encoder = context.device().create_command_encoder(&Default::default());
        let mut recording = renderer.recording(encoder)?;
        recording.encode(&prepared, attachment(&target))?;
        let recorded = recording.finish()?;
        renderer.submit(recorded)?;
        context.device().poll(wgpu::PollType::wait_indefinitely())?;
        let stats = renderer.stats();
        assert!(stats.buffer_slots <= 2);
        if count == 33 {
            retained = stats.buffer_bytes;
        } else if count > 33 {
            assert_eq!(stats.buffer_bytes, retained);
        }
    }
    Ok(())
}

fn tight_budget_preserves_the_first_view(gpu: &mut Gpu) -> Result {
    let context = gpu.context();
    let mut renderer = renderer_with_budgets(
        context,
        Budgets {
            max_buffer_bytes: 272,
            ..Default::default()
        },
    )?;
    let mut scene = DrawList::new();
    for _ in 0..4 {
        Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), Color::WHITE);
    }
    let _ = renderer.prepare(&scene)?;
    let encoder = context.device().create_command_encoder(&Default::default());
    let recorded = renderer.recording(encoder)?.finish()?;
    renderer.cancel(recorded)?;
    scene.clear();
    for _ in 0..3 {
        Canvas::new(&mut scene).fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), Color::WHITE);
    }
    let prepared = renderer.prepare(&scene)?;
    let prepared = renderer.prepare_view(&prepared, view(), CONFIG)?;
    let target = target(context).create_view(&Default::default());
    let encoder = context.device().create_command_encoder(&Default::default());
    let mut recording = renderer.recording(encoder)?;
    recording.encode(&prepared, attachment(&target))?;
    let recorded = recording.finish()?;
    renderer.submit(recorded)?;
    context.device().poll(wgpu::PollType::wait_indefinitely())?;
    assert!(renderer.stats().buffer_bytes <= 272);
    Ok(())
}

fn run(test: fn(&mut Gpu) -> Result) -> std::result::Result<(), String> {
    gpu_test::with_shared(Capability::Compute, |gpu| Ok(test(gpu)))
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

fn main() -> std::process::ExitCode {
    type Case = (&'static str, fn(&mut Gpu) -> Result);
    let cases: [Case; 19] = [
        ("transparent_order", transparent_order),
        (
            "cancelled_upload_can_be_retried",
            cancelled_upload_can_be_retried,
        ),
        (
            "prepared_view_reuses_geometry_across_completed_batches",
            prepared_view_reuses_geometry_across_completed_batches,
        ),
        (
            "recorded_atlas_copy_keeps_its_page_until_cancel",
            recorded_atlas_copy_keeps_its_page_until_cancel,
        ),
        (
            "batched_colors_match_linear_coverage",
            batched_colors_match_linear_coverage,
        ),
        (
            "image_uses_premultiplied_alpha",
            image_uses_premultiplied_alpha,
        ),
        (
            "rounded_rect_excludes_corners",
            rounded_rect_excludes_corners,
        ),
        ("border_leaves_interior_empty", border_leaves_interior_empty),
        (
            "image_keeps_its_painter_position",
            image_keeps_its_painter_position,
        ),
        (
            "removed_image_key_is_rejected",
            removed_image_key_is_rejected,
        ),
        (
            "clip_excludes_external_pixels",
            clip_excludes_external_pixels,
        ),
        (
            "multiview_keeps_independent_uniforms",
            multiview_keeps_independent_uniforms,
        ),
        (
            "recorded_commands_keep_geometry_alive",
            recorded_commands_keep_geometry_alive,
        ),
        (
            "glyph_masks_preserve_premultiplied_coverage",
            glyph_masks_preserve_premultiplied_coverage,
        ),
        (
            "glyph_keys_require_their_font_system",
            glyph_keys_require_their_font_system,
        ),
        (
            "glyphs_require_the_text_entry",
            glyphs_require_the_text_entry,
        ),
        (
            "clipped_long_text_keeps_a_visible_glyph_workset",
            clipped_long_text_keeps_a_visible_glyph_workset,
        ),
        (
            "growing_geometry_reuses_upload_capacity",
            growing_geometry_reuses_upload_capacity,
        ),
        (
            "tight_budget_preserves_the_first_view",
            tight_budget_preserves_the_first_view,
        ),
    ];
    gpu_test::run(
        cases
            .into_iter()
            .map(|(name, test)| {
                Trial::test(name, move || run(test).map_err(Into::into))
                    .with_kind("2d")
                    .with_ignored_flag(true)
            })
            .collect(),
    )
}
