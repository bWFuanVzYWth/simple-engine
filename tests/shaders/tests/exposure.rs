//! 自动曝光生产录制器的真实 GPU 回归；只用附加 shader 回读设备内状态。

use simple_engine::{
    display::exposure::{
        Frame,
        gpu::{Exposure, Programs},
    },
    gpu_test::{Capability, Gpu},
    vulkan::{
        interop::RayShader,
        native::{Binding, Bindings, Buffer, Commands, ComputePipeline},
    },
    wgpu,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const HISTOGRAM: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/exposure.histogram.spv")),
    entry: c"histogram_main",
};
const UPDATE: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/exposure.update.spv")),
    entry: c"update_main",
};
const READBACK: RayShader<'static> = RayShader {
    spirv: include_bytes!(concat!(env!("OUT_DIR"), "/exposure.readback.spv")),
    entry: c"readback_main",
};

struct Fixture {
    gpu: Gpu,
    exposure: Exposure,
    input: Buffer,
    parameters: Buffer,
    output: Buffer,
    readback: ComputePipeline,
    extent: [u32; 2],
}

impl Fixture {
    fn new(extent: [u32; 2]) -> Result<Self> {
        let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
        let context = gpu.context_mut().native();
        let input = Buffer::new(
            context,
            u64::from(extent[0]) * u64::from(extent[1]) * 16,
            true,
        )?;
        let parameters = Buffer::new(context, 32, true)?;
        let exposure = Exposure::new(
            context,
            Programs {
                histogram: HISTOGRAM,
                update: UPDATE,
            },
            Binding::StorageBuffer(input),
            Binding::StorageBuffer(parameters),
            extent,
        )?;
        let output = Buffer::new(context, 16, true)?;
        let bindings = Bindings::new(
            context,
            &[
                Binding::StorageBuffer(exposure.state()),
                Binding::StorageBuffer(output),
            ],
        )?;
        let readback = ComputePipeline::new(context, bindings, READBACK)?;
        Ok(Self {
            gpu,
            exposure,
            input,
            parameters,
            output,
            readback,
            extent,
        })
    }

    fn frame(&self, dt: f32, reset: bool, instant: bool, strength: f32, bt709: bool) -> Frame {
        Frame::new(self.extent, bt709, dt, reset, instant, strength).unwrap()
    }

    fn step(&mut self, pixels: &[[f32; 4]], frame: Frame) -> Result<[u32; 4]> {
        assert_eq!(
            pixels.len(),
            self.extent[0] as usize * self.extent[1] as usize
        );
        let pixels: Vec<_> = pixels
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let parameters: Vec<_> = frame
            .to_words()
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        let context = self.gpu.context_mut();
        self.input.write(context.native(), 0, &pixels)?;
        self.parameters.write(context.native(), 0, &parameters)?;
        let recorded = self.exposure.record(context.native())?;
        let exposure = context.record_native(recorded)?;
        let mut commands = Commands::new(context.native())?;
        commands.barrier();
        commands.dispatch(self.readback, [1; 3])?;
        commands.barrier();
        let recorded = commands.finish(context.native())?;
        let readback = context.record_native(recorded)?;
        let submission = context.submit([exposure, readback])?;
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        let mut bytes = [0; 16];
        self.output.read(context.native(), 0, &mut bytes)?;
        Ok(std::array::from_fn(|i| {
            u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
        }))
    }
}

fn with_fixture<T>(extent: [u32; 2], run: impl FnOnce(&mut Fixture) -> Result<T>) -> Result<T> {
    let mut fixture = Fixture::new(extent)?;
    let result = run(&mut fixture);
    let cleanup = fixture.gpu.finish();
    let value = result?;
    cleanup?;
    Ok(value)
}

fn close(actual: f32, expected: f64) {
    assert!(
        actual.is_finite() && (f64::from(actual) - expected).abs() <= 2e-5,
        "GPU={actual}, reference={expected}"
    );
}

fn bin_center(brightness: f64) -> f64 {
    let logarithm = brightness.max(2.0f64.powi(-16)).log2().clamp(-16.0, 20.0);
    let bin = (((logarithm + 16.0) * 256.0 / 36.0).floor() as u32).min(255);
    -16.0 + (f64::from(bin) + 0.5) * 36.0 / 256.0
}

fn gray_ev(brightness: f64) -> f64 {
    (0.16f64.log2() - bin_center(brightness)).clamp(-16.0, 16.0)
}

#[test]
#[ignore = "需要 Vulkan compute；执行生产直方图与更新阶段"]
fn first_frame_meters_uniform_gray_across_partial_tiles() -> Result {
    with_fixture([65, 3], |fixture| {
        let pixels = vec![[0.125, 0.125, 0.125, 1.0]; 65 * 3];
        let state = fixture.step(&pixels, fixture.frame(0.0, false, false, 1.0, false))?;
        assert_eq!(state[1], 1);
        close(f32::from_bits(state[0]), gray_ev(0.125));
        close(f32::from_bits(state[2]), gray_ev(0.125));
        close(f32::from_bits(state[3]), bin_center(0.125));
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；两端裁尾不受孤立极值影响"]
fn histogram_tails_remove_isolated_black_and_highlights() -> Result {
    with_fixture([25, 16], |fixture| {
        let mut pixels = vec![[0.125, 0.125, 0.125, 1.0]; 400];
        pixels[0] = [0.0, 0.0, 0.0, 1.0];
        pixels[399] = [1e30, 1e30, 1e30, 1.0];
        let state = fixture.step(&pixels, fixture.frame(0.0, false, false, 1.0, false))?;
        close(f32::from_bits(state[0]), gray_ev(0.125));
        close(f32::from_bits(state[3]), bin_center(0.125));
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；零间隔保留曝光但更新目标"]
fn zero_delta_holds_exposure_while_target_tracks_scene() -> Result {
    with_fixture([1, 1], |fixture| {
        let first = fixture.step(
            &[[0.125, 0.125, 0.125, 1.0]],
            fixture.frame(0.0, false, false, 1.0, false),
        )?;
        let changed = fixture.step(
            &[[8.0, 8.0, 8.0, 1.0]],
            fixture.frame(0.0, false, false, 1.0, false),
        )?;
        assert_eq!(changed[0], first[0]);
        close(f32::from_bits(changed[2]), gray_ev(8.0));
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；暗化与亮化的 T90 不同"]
fn adaptation_reaches_ninety_percent_at_each_t90() -> Result {
    with_fixture([1, 1], |fixture| {
        let low = [[0.125, 0.125, 0.125, 1.0]];
        let high = [[8.0, 8.0, 8.0, 1.0]];
        let initial = fixture.step(&low, fixture.frame(0.0, true, false, 1.0, false))?;
        let darkened = fixture.step(&high, fixture.frame(0.5, false, false, 1.0, false))?;
        close(
            f32::from_bits(darkened[0]),
            f64::from(f32::from_bits(initial[0])) * 0.1 + gray_ev(8.0) * 0.9,
        );
        fixture.step(&high, fixture.frame(0.0, true, false, 1.0, false))?;
        let brightened = fixture.step(&low, fixture.frame(2.0, false, false, 1.0, false))?;
        close(
            f32::from_bits(brightened[0]),
            gray_ev(8.0) * 0.1 + gray_ev(0.125) * 0.9,
        );
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；空有效样本保留状态，显式重置归零"]
fn invalid_pixels_do_not_change_history_until_reset() -> Result {
    with_fixture([65, 3], |fixture| {
        let pixels = vec![[0.125, 0.125, 0.125, 1.0]; 195];
        let initial = fixture.step(&pixels, fixture.frame(0.0, false, false, 1.0, false))?;
        let invalid: Vec<_> = (0..195)
            .map(|i| match i % 3 {
                0 => [f32::NAN, 1.0, 1.0, 1.0],
                1 => [1.0, f32::INFINITY, 1.0, 1.0],
                _ => [8.0, 8.0, 8.0, -1.0],
            })
            .collect();
        let held = fixture.step(&invalid, fixture.frame(1.0, false, false, 1.0, false))?;
        assert_eq!(held, initial);
        let reset = fixture.step(&invalid, fixture.frame(1.0, true, false, 1.0, false))?;
        assert_eq!(reset, [0, 1, 0, 0]);
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；自动调节强度在 EV 域缩放"]
fn compensation_interpolates_between_neutral_and_metered_ev() -> Result {
    with_fixture([1, 1], |fixture| {
        let pixels = [[0.125, 0.125, 0.125, 1.0]];
        for strength in [0.0, 0.5, 1.0] {
            let state = fixture.step(&pixels, fixture.frame(0.0, false, true, strength, false))?;
            close(
                f32::from_bits(state[0]),
                gray_ev(0.125) * f64::from(strength),
            );
        }
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；BT.709 与对应 Rec.2020 原色计量一致"]
fn bt709_metering_matches_rec2020_conversion() -> Result {
    with_fixture([1, 1], |fixture| {
        let bt709 = fixture.step(
            &[[1.0, 0.0, 0.0, 1.0]],
            fixture.frame(0.0, false, true, 1.0, true),
        )?;
        let rec2020 = fixture.step(
            &[[0.627_403_9, 0.069_097_29, 0.016_391_44, 1.0]],
            fixture.frame(0.0, false, true, 1.0, false),
        )?;
        assert_eq!(bt709, rec2020);
        let red_brightness = 0.627403896 * 0.2627 + 0.069097289 * 0.6780 + 0.016391439 * 0.0593;
        close(f32::from_bits(bt709[0]), gray_ev(red_brightness));
        Ok(())
    })
}

#[test]
#[ignore = "需要 Vulkan compute；场景直方图的明暗倾向参与目标 EV"]
fn scene_key_preserves_skewed_brightness_distribution() -> Result {
    with_fixture([100, 1], |fixture| {
        let mut pixels = vec![[0.125, 0.125, 0.125, 1.0]; 75];
        pixels.extend([[8.0, 8.0, 8.0, 1.0]; 25]);
        let state = fixture.step(&pixels, fixture.frame(0.0, false, false, 1.0, false))?;
        let minimum = bin_center(0.125);
        let maximum = bin_center(8.0);
        let mean = minimum * 0.75 + maximum * 0.25;
        close(f32::from_bits(state[3]), mean);
        close(f32::from_bits(state[0]), 0.16f64.log2() - mean - 1.0);
        Ok(())
    })
}
