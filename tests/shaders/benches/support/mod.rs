use simple_engine::{
    gpu_test::{self, Gpu, Sample},
    vulkan::{self, native::Commands},
};
use std::time::Instant;

pub(super) fn native_frames(
    gpu: &mut Gpu,
    frames: u32,
    mut record_frame: impl FnMut(&mut Commands) -> Result<(), vulkan::Error>,
) -> Result<(Sample, f64), gpu_test::Error> {
    let start = Instant::now();
    let context = gpu.context_mut();
    let native = context.native();
    let mut commands = Commands::new(native)?;
    for _ in 0..frames {
        record_frame(&mut commands)?;
    }
    let recorded = commands.finish(native)?;
    let commands = context.record_native(recorded)?;
    let record_seconds = start.elapsed().as_secs_f64();
    Ok((gpu.measure(vec![commands])?, record_seconds))
}
