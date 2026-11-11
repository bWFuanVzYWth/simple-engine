//! 固定 stage 资源编号，使 native SPIR-V 与 wgpu 的紧密 binding 布局一致。

#[cfg(not(any(feature = "gpu", feature = "native")))]
fn main() {}

#[cfg(any(feature = "gpu", feature = "native"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use shader_build::{Recipe, Target, build};
    let stages = [
        ("optical_depth", "ATM_STAGE_OPTICAL"),
        ("prepare_directions", "ATM_STAGE_DIRECTIONS"),
        ("trace_incident", "ATM_STAGE_INCIDENT"),
        ("incident_moments", "ATM_STAGE_MOMENTS"),
        ("scattering_source", "ATM_STAGE_SOURCE"),
        ("ground_irradiance", "ATM_STAGE_GROUND"),
        ("build_sky", "ATM_STAGE_SKY"),
        ("build_aerial", "ATM_STAGE_AERIAL"),
        ("transport_probe", "ATM_STAGE_PROBE"),
    ];
    let outputs: Vec<_> = stages
        .iter()
        .map(|(entry, _)| format!("{entry}.spv"))
        .collect();
    let definitions: Vec<_> = stages
        .iter()
        .map(|(_, name)| format!("-D{name}=1"))
        .collect();
    let arguments: Vec<_> = definitions
        .iter()
        .map(|definition| [definition.as_str()])
        .collect();
    let mut recipes: Vec<_> = stages
        .iter()
        .enumerate()
        .map(|(index, (entry, _))| Recipe {
            source: "../../shaders/atmosphere/compute.slang",
            entry,
            stage: "compute",
            output: &outputs[index],
            target: Target::Spirv("spirv_1_3"),
            args: &arguments[index],
        })
        .collect();
    if cfg!(feature = "native") {
        for (source, entry, output, args) in [
            (
                "../../shaders/atmosphere/shadow_prepare.slang",
                "shadow_prepare",
                "shadow_prepare.spv",
                &[][..],
            ),
            (
                "../../shaders/atmosphere/shadow_demand.slang",
                "shadow_demand",
                "shadow_demand.spv",
                &[][..],
            ),
            (
                "../../shaders/atmosphere/compute.slang",
                "terrain_integrate",
                "terrain_integrate.spv",
                &["-DATM_STAGE_TERRAIN=1"][..],
            ),
        ] {
            recipes.push(Recipe {
                source,
                entry,
                stage: "compute",
                output,
                target: Target::Spirv("spirv_1_3"),
                args,
            });
        }
    }
    build(&recipes, &["../../shaders"], &["src", "data"])?;
    Ok(())
}
