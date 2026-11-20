//! 真机执行生产录制器，按未填充图像的空间卷积核对输出。

use simple_engine::{
    circular_bloom,
    fft_bloom::{Kernel, MAX_RADIUS, Settings, gpu::Bloom},
    gpu_test::{Capability, Gpu},
    vulkan::{
        self,
        native::{Commands, Context, Image, ImageFormat, ImagePasses},
    },
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn render_frames(
    extent: [u32; 2],
    frames: &[&[[f32; 4]]],
    kernel: &Kernel,
    settings: Settings,
) -> Result<Vec<[f32; 4]>> {
    render_with(extent, frames, |ctx, input| {
        Bloom::new(
            ctx,
            simple_engine::fft_bloom::gpu::PROGRAMS,
            input,
            kernel,
            settings,
        )
        .map(Into::into)
    })
}

fn render_with(
    extent: [u32; 2],
    frames: &[&[[f32; 4]]],
    bind: impl FnOnce(&mut Context, Image) -> std::result::Result<ImagePasses, vulkan::Error>,
) -> Result<Vec<[f32; 4]>> {
    assert!(!frames.is_empty());
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let native = gpu.context_mut().native();
    let input = Image::new(native, [extent[0], extent[1], 1], ImageFormat::Rgba32Float)?;
    let bloom = bind(native, input)?;
    native.collect(&bloom.roots().collect::<Vec<_>>())?;
    for &pixels in frames {
        input.upload(native, bytemuck::cast_slice(pixels))?;
        let mut commands = Commands::new(native)?;
        bloom.record_into(&mut commands)?;
        commands.finish(native)?.submit_and_wait(native)?;
    }
    let result = bloom.output().download(native)?;
    gpu.finish()?;
    Ok(result
        .as_chunks::<16>()
        .0
        .iter()
        .copied()
        .map(bytemuck::cast)
        .collect())
}

#[test]
#[ignore = "需要 Vulkan compute，核对非对称卷积与分块边缘"]
fn asymmetric_bloom_matches_spatial_convolution_across_tiles() -> Result {
    let [width, height] = [269usize, 261];
    let pixels: Vec<_> = (0..width * height)
        .map(|i| {
            let (x, y) = (i % width, i / width);
            [
                ((x + 3 * y) % 17) as f32 * 0.25,
                ((7 * x + y) % 13) as f32 * 0.5 - 1.0,
                (x ^ y) as u8 as f32 / 128.0,
                (x % 4) as f32 * 0.25,
            ]
        })
        .collect();
    let weights = [1., 0., 0., 3., 0., 0., 0., 4., 0., 0., 0., 2., 0., 0., 0.];
    let kernel = Kernel::new([5, 3], weights.to_vec())?;
    let settings = Settings::new(1.0, 0.75)?;
    let result = render_frames([width as u32, height as u32], &[&pixels], &kernel, settings)?;

    // 从每个亮点向目标像素分光，画外贡献自然丢弃。
    let mut expected: Vec<_> = pixels.iter().map(|p| p.map(f64::from)).collect();
    let mass: f64 = weights.iter().copied().map(f64::from).sum();
    for (i, pixel) in pixels.iter().enumerate() {
        let [r, g, b, _] = pixel.map(f64::from);
        let peak = r.max(g).max(b);
        if peak <= 1.0 {
            continue;
        }
        for (k, &weight) in weights.iter().enumerate() {
            let x = (i % width) as isize + (k % 5) as isize - 2;
            let y = (i / width) as isize + (k / 5) as isize - 1;
            if !(0..width as isize).contains(&x) || !(0..height as isize).contains(&y) {
                continue;
            }
            let gain = 0.75 * (1.0 - 1.0 / peak) * f64::from(weight) / mass;
            let target = &mut expected[y as usize * width + x as usize];
            for (channel, source) in target.iter_mut().zip([r, g, b]) {
                *channel += gain * source.max(0.0);
            }
        }
    }
    assert_eq!(result.len(), expected.len());
    for (i, (actual, expected)) in result.iter().zip(expected).enumerate() {
        assert_eq!(actual[3], pixels[i][3]);
        for c in 0..3 {
            assert!(
                (f64::from(actual[c]) - expected[c]).abs() < 2e-5,
                "像素 {i} 通道 {c} 的 GPU 值为 {}，参考值为 {}",
                actual[c],
                expected[c]
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查最大光斑半径与画外补零"]
fn corner_impulse_only_lights_the_visible_quadrant() -> Result {
    let kernel = Kernel::gaussian(MAX_RADIUS, 24.0)?;
    let [width, height] = [269usize, 271];
    let mut pixels = vec![[0.0, 0.0, 0.0, 1.0]; width * height];
    pixels[0][2] = 20.0;
    let result = render_frames(
        [width as u32, height as u32],
        &[&pixels],
        &kernel,
        Settings::new(0.0, 1.0)?,
    )?;
    let radius = MAX_RADIUS as usize;
    for (i, actual) in result.iter().enumerate() {
        let (x, y) = (i % width, i / width);
        let expected = if x <= radius && y <= radius {
            20.0 * kernel.channels()[0][(y + radius) * (2 * radius + 1) + x + radius]
        } else {
            0.0
        };
        assert!(
            (actual[2] - pixels[i][2] - expected).abs() < 2e-6,
            "像素 {i} 的角点响应不符"
        );
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查无效输入的作用范围"]
fn invalid_pixels_stay_local() -> Result {
    let mut pixels = [[0.25, -0.125, 0.5, 0.75]; 35];
    pixels[9] = [4.0, 32.0, 8.0, -1.0];
    pixels[17] = [f32::NAN, 8.0, 16.0, 1.0];
    let result = render_frames(
        [7, 5],
        &[&pixels],
        &Kernel::gaussian(2, 1.25)?,
        Settings::default(),
    )?;
    for (actual, original) in result.iter().zip(pixels) {
        for (value, expected) in actual.iter().zip(original) {
            assert!(*value == expected || (value.is_nan() && expected.is_nan()));
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查逐帧复用"]
fn moving_highlight_replaces_the_previous_halo() -> Result {
    let dark = [[0.0, 0.0, 0.0, 1.0]; 5];
    let mut first = dark;
    first[0][0] = 4.0;
    let mut second = dark;
    second[2][1] = 3.0;
    let kernel = Kernel::new([3, 1], vec![0.0, 0.0, 1.0])?;
    let result = render_frames(
        [5, 1],
        &[&first, &second],
        &kernel,
        Settings::new(1.0, 1.0)?,
    )?;
    let mut expected = second;
    expected[3][1] = 2.0;
    for (actual, expected) in result.iter().zip(expected) {
        for (value, target) in actual.iter().zip(expected) {
            assert!((value - target).abs() < 1e-6);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查各通道独立的有符号光学卷积"]
fn optical_filter_moves_each_signed_channel_independently() -> Result {
    let kernel = Kernel::from_rgb(
        [3, 1],
        [vec![1., 0., 0.], vec![0., 1., 0.], vec![0., 0., 1.]],
    )?;
    let mut pixels = [[0., 0., 0., 0.75]; 5];
    pixels[2] = [8., -4., 2., 0.5];
    let result = render_frames([5, 1], &[&pixels], &kernel, Settings::OPTICAL)?;
    let mut expected = [[0., 0., 0., 0.75]; 5];
    expected[1][0] = 8.;
    expected[2] = [0., -4., 0., 0.5];
    expected[3][2] = 2.;
    for (pixel, reference) in result.iter().zip(expected) {
        for (&actual, target) in pixel.iter().zip(reference) {
            assert!((actual - target).abs() < 2e-6);
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查宽 FFT 的非对称有符号卷积和跨块写入"]
fn wide_filter_moves_signed_impulses_across_the_tile_boundary() -> Result {
    let mut weights = std::array::from_fn(|_| vec![0.; 1921]);
    for (channel, offset) in weights.iter_mut().zip([800, -600, 0]) {
        channel[(960 + offset) as usize] = 1.;
    }
    let kernel = Kernel::from_rgb([1921, 1], weights)?;
    let mut pixels = vec![[0., 0., 0., 0.5]; 2603];
    pixels[1500] = [5., -3., 9., 0.5];
    let output = render_frames([2603, 1], &[&pixels], &kernel, Settings::OPTICAL)?;
    let mut expected = vec![[0., 0., 0., 0.5]; pixels.len()];
    expected[2300][0] = 5.;
    expected[900][1] = -3.;
    expected[1500][2] = 9.;
    for (actual, reference) in output.iter().zip(expected) {
        assert!(
            actual
                .iter()
                .zip(reference)
                .all(|(a, b)| (a - b).abs() < 2e-5)
        );
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查更换 FFT 矩形后两种视图的有符号远翼与补零边界"]
fn asymmetric_wings_remain_valid_across_view_resizes() -> Result {
    use simple_engine::fft_bloom::gpu::{Filter, PROGRAMS};
    // 不同颜色沿各自的位移分光。长轴强制分块，短视图裁去无法在画内到达的位移。
    let radius = 912;
    let width = 2 * radius + 1;
    let offsets = [(912_i32, -2_i32), (-511, 1), (41, 2)];
    let channels = offsets.map(|(dx, dy)| {
        let mut w = vec![0.; 5 * width];
        w[(2 + dy) as usize * width + (radius as i32 + dx) as usize] = 1.;
        w
    });
    let kernel = Kernel::from_rgb([width as u32, 5], channels)?;
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let native = gpu.context_mut().native();
    let mut filter = Filter::new(native, PROGRAMS, &kernel, Settings::OPTICAL)?;
    let mut views = Vec::new();
    for [w, h] in [[7801usize, 9usize], [83, 6201], [83, 5]] {
        let input = Image::new(native, [w as u32, h as u32, 1], ImageFormat::Rgba32Float)?;
        let mut pixels = vec![[0., 0., 0., 0.75]; w * h];
        let mut expected = pixels.clone();
        for [x, y] in [[0, 0], [w / 2, h / 2], [w - 1, h - 1]] {
            let rgb = [1., -3., 7.];
            pixels[y * w + x][..3].copy_from_slice(&rgb);
            for (c, (dx, dy)) in offsets.into_iter().enumerate() {
                let tx = x as i32 + dx;
                let ty = y as i32 + dy;
                if (0..w as i32).contains(&tx) && (0..h as i32).contains(&ty) {
                    expected[ty as usize * w + tx as usize][c] += rgb[c];
                }
            }
        }
        pixels[1] = [80., 30., -2., -1.];
        expected[1] = pixels[1];
        input.upload(native, bytemuck::cast_slice(&pixels))?;
        views.push((filter.bind(native, input)?, expected));
    }
    native.collect(
        &filter
            .roots()
            .chain(views.iter().flat_map(|(view, _)| view.roots()))
            .collect::<Vec<_>>(),
    )?;
    for (view, expected) in views {
        let mut commands = Commands::new(native)?;
        view.record_into(&mut commands)?;
        commands.finish(native)?.submit_and_wait(native)?;
        let out = view.output().download(native)?;
        for (bytes, target) in out.as_chunks::<16>().0.iter().zip(expected) {
            let value: [f32; 4] = bytemuck::cast(*bytes);
            assert!(
                value.iter().zip(target).all(|(a, b)| (a - b).abs() < 2e-5),
                "{value:?} / {target:?}"
            );
        }
    }
    gpu.finish()?;
    Ok(())
}

fn circular(extent: [u32; 2], pixels: &[[f32; 4]]) -> Result<Vec<[f32; 4]>> {
    render_with(extent, &[pixels], |ctx, input| {
        circular_bloom::Filter::new(ctx)?.bind(ctx, input)
    })
}

// 用解析包围能量独立算方形中心像素。八分之一圆周上，半径止于 x=0.5 的像素边。
fn airy_center(band: [u16; 2]) -> f64 {
    let mut sum = 0.;
    for k in 0..32 {
        let nm = f64::from(band[0]) + f64::from(band[1] - band[0]) * (k as f64 + 0.5) / 32.;
        for a in 0..96 {
            let angle = std::f64::consts::FRAC_PI_4 * (a as f64 + 0.5) / 96.;
            let x = std::f64::consts::PI * 0.5 / (angle.cos() * nm * 0.001 * 4. / 5.);
            sum += 1. - libm::j0(x).powi(2) - libm::j1(x).powi(2);
        }
    }
    sum / (32. * 96.)
}

#[test]
#[ignore = "需要 Vulkan compute，用解析圆孔核核对原像素中心、不同格点相位和远翼圆度"]
fn circular_impulses_follow_broadband_airy() -> Result {
    let [w, h] = [997usize, 961];
    let core = [[590, 690], [500, 590], [420, 500]].map(airy_center);
    for (cx, cy) in [(486, 462), (499, 487), (511, 476), (480, 481)] {
        let mut pixels = vec![[0., 0., 0., 1.]; w * h];
        pixels[cy * w + cx] = [2_097_152., -1_048_576., 524_288., 0.375];
        let input = pixels[cy * w + cx];
        let out = circular([w as u32, h as u32], &pixels)?;
        assert_eq!(out[cy * w + cx][3], input[3]);
        for c in 0..3 {
            let center = f64::from(out[cy * w + cx][c] / input[c]);
            assert!(
                (center - core[c]).abs() < 0.0008,
                "像素相位 ({cx},{cy}) 通道 {c} 的中心 {center} / {}",
                core[c]
            );
            let sum = out
                .iter()
                .map(|p| f64::from(p[c]) / f64::from(input[c]))
                .sum::<f64>();
            assert!((0.9997..=1.000001).contains(&sum), "有限视图通量 {sum}");
        }
        // 3-4-5 三角形给出同一半径的整数格点，避免用插值掩盖方位差。
        for unit in [4usize, 12, 32] {
            let r = 5 * unit;
            let at = |dx: usize, dy: usize| f64::from(out[(cy + dy) * w + cx + dx][1] / input[1]);
            let spokes = [
                at(r, 0),
                at(4 * unit, 3 * unit),
                at(3 * unit, 4 * unit),
                at(0, r),
            ];
            let min = spokes.into_iter().fold(f64::INFINITY, f64::min);
            let max = spokes.into_iter().fold(0., f64::max);
            let airy = 0.545 * 4. / 5. / (std::f64::consts::PI * r as f64).powi(3);
            assert!(
                max / min < 1.15 && min / airy > 0.7 && max / airy < 1.3,
                "半径 {r} 的方向响应 {spokes:?}，Airy {airy}"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，检查圆形滤波的黑色画外和无效样本"]
fn circular_black_extension_survives_odd_and_tiny_crops() -> Result {
    // 保持原点相位相同，在大黑图和较小视图的左上角观察同一亮点。
    let render = |w: usize, h: usize| {
        let mut pixels = vec![[0., 0., 0., 1.]; w * h];
        pixels[0] = [120_000., -0.75, 0.25, 0.5];
        if w > 1 {
            pixels[1] = [8192., f32::NAN, -13., 1.];
        }
        if h > 1 {
            pixels[w] = [999_999., 12., 4., -1.];
        }
        circular([w as u32, h as u32], &pixels)
    };
    let large = render(131, 97)?;
    for (w, h) in [(1, 1), (3, 1), (37, 29)] {
        let out = render(w, h)?;
        for y in 0..h {
            for x in 0..w {
                let [r, g, b, a] = out[y * w + x];
                if (x, y) == (1, 0) {
                    assert!(g.is_nan() && [r, b, a] == [8192., -13., 1.]);
                } else if (x, y) == (0, 1) {
                    assert_eq!([r, g, b, a], [999_999., 12., 4., -1.]);
                } else {
                    assert_eq!(
                        out[y * w + x],
                        large[y * 131 + x],
                        "画外补黑后 ({x},{y}) 的结果应与大图相同"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute，核对快速圆形视图对每一帧重新建立全部远翼"]
fn circular_view_replaces_its_previous_pyramid() -> Result {
    let extent = [131, 97];
    let mut first = vec![[0., 0., 0., 1.]; 131 * 97];
    first[41 * 131 + 64] = [134_217_728., -65000., 600., 0.25];
    let mut next = vec![[0., 0., 0., 1.]; first.len()];
    next[60 * 131 + 27] = [0.5, 0.03, -4., 0.5];
    let actual = render_with(extent, &[&first, &next], |ctx, input| {
        circular_bloom::Filter::new(ctx)?.bind(ctx, input)
    })?;
    assert_eq!(actual, circular(extent, &next)?);
    Ok(())
}
