//! ReSTIR生产shader的位精确存储、候选域与热冷合并行为。
#![forbid(unsafe_code)]

use simple_engine::{
    gpu_log,
    vulkan::{
        interop::{RayShader, WgpuContext},
        native::{self, Binding},
    },
};
use std::error::Error;

fn compute(shader: RayShader<'_>, input: &[u8], bytes: usize) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut context = pollster::block_on(WgpuContext::compute())?;
    let source = native::Buffer::upload(context.native(), input)?;
    let output = native::Buffer::new(context.native(), bytes as u64, true)?;
    let bindings = native::Bindings::new(
        context.native(),
        &[
            Binding::StorageBuffer(source),
            Binding::StorageBuffer(output),
        ],
    )?;
    let program = native::ComputePipeline::new(context.native(), bindings, shader)?;
    let mut commands = native::Commands::new(context.native())?;
    commands.dispatch(program, [1; 3])?;
    commands
        .finish(context.native())?
        .submit_and_wait(context.native())?;
    let mut actual = vec![0; bytes];
    output.read(context.native(), 0, &mut actual)?;
    Ok(actual)
}

#[test]
#[ignore = "需要真实Vulkan；由主任务独占GPU运行"]
fn raw_records_preserve_every_bit_without_padding() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let reconnection_start = 55 * 80;
        let precomputed_start = reconnection_start + 55 * 44;
        let bytes = precomputed_start + 55 * 20;
        let input: Vec<_> = (0..bytes / 4)
            .flat_map(|index| {
                let patterns = [0u32, 0x8000_0000, 0x7f80_0000, 0x7fc1_2345, 1, 0x7f7f_ffff];
                let word = if index % 7 < patterns.len() {
                    patterns[index % 7]
                } else {
                    (index as u32).wrapping_mul(2_654_435_761)
                };
                word.to_le_bytes()
            })
            .collect();
        let actual = compute(
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/restir_storage.spv")),
                entry: c"storage_roundtrip",
            },
            &input,
            bytes,
        )?;
        assert_eq!(actual, input);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实Vulkan；由主任务独占GPU运行"]
fn admission_is_symmetric_and_keeps_thin_wall_support() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let actual = compute(
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/restir_admission.spv")),
                entry: c"admission_cases",
            },
            &[0; 4],
            40,
        )?;
        let words: Vec<_> = actual
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word))
            .collect();
        assert_eq!(words, [1, 1, 0, 1, 0, 12, 9, 1, 0, 0]);
        Ok(())
    })
}

#[test]
#[ignore = "需要真实Vulkan；由主任务独占GPU运行"]
fn hot_selection_restores_the_same_cold_winner() -> Result<(), Box<dyn Error>> {
    gpu_log::checked(|| {
        let mut input = Vec::new();
        for seed in [7u32, 29] {
            let mut words = [0; 20];
            words[0] = (if seed == 7 { 1f32 } else { 32f32 }).to_bits();
            words[1] = 0.5f32.to_bits();
            words[2..5].fill(0.7f32.to_bits());
            words[5] = 0x0040_0203;
            words[6] = seed;
            words[7] = seed * 3;
            words[8] = 0.2f32.to_bits();
            words[9..13].fill(seed);
            words[13] = 0.9f32.to_bits();
            words[14..17].fill(0.3f32.to_bits());
            words[17..20].fill(0.4f32.to_bits());
            input.extend(words.into_iter().flat_map(u32::to_le_bytes));
        }
        let actual = compute(
            RayShader {
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/restir_selection.spv")),
                entry: c"selection_equivalence",
            },
            &input,
            256 * 80,
        )?;
        for (index, pair) in actual.as_chunks::<160>().0.iter().enumerate() {
            assert_eq!(&pair[..80], &pair[80..], "selection case {index}");
        }
        Ok(())
    })
}
