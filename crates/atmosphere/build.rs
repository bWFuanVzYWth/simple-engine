//! 固定 stage 资源编号，使 native SPIR-V 与 wgpu 的紧密 binding 布局一致。

#[cfg(not(any(feature = "gpu", feature = "native")))]
fn main() {}

#[cfg(any(feature = "gpu", feature = "native"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use shader_build::{Recipe, Target, build};
    use std::{env, fmt::Write, fs, path::PathBuf};

    // 索引对应 compute.slang 的 B0..B23，顺序就是各入口的紧密 binding 编号。
    let mut stages: Vec<(&str, &str, &[u32])> = vec![
        ("compute", "optical_depth", &[0, 1, 11]),
        ("compute", "prepare_directions", &[0, 1, 16]),
        (
            "compute",
            "trace_incident",
            &[0, 1, 2, 3, 4, 5, 6, 16, 7, 18],
        ),
        ("compute", "incident_moments", &[0, 16, 7, 17, 19, 9]),
        ("compute", "scattering_source", &[0, 1, 8, 4, 16, 7, 17]),
        ("compute", "ground_irradiance", &[0, 1, 2, 6, 16, 7, 10]),
        ("compute", "build_sky", &[0, 1, 2, 3, 4, 5, 6, 18, 20, 14]),
        (
            "compute",
            "build_aerial",
            &[0, 1, 2, 3, 4, 5, 6, 18, 21, 22, 23],
        ),
        (
            "transport_probe",
            "transport_probe",
            &[0, 1, 2, 3, 4, 5, 6, 18],
        ),
    ];
    if cfg!(feature = "native") {
        stages.push((
            "terrain_integrate",
            "terrain_integrate",
            &[0, 1, 2, 3, 4, 5, 6, 18],
        ));
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo 未设置 OUT_DIR")?);
    let mut sources = Vec::new();
    let mut outputs = Vec::new();
    let mut arguments = Vec::new();
    for (source, entry, resources) in &stages {
        let directory = output.join(entry);
        fs::create_dir_all(&directory)?;
        let mut bindings = String::from("module atmosphere_bindings;\n");
        for index in 0..24 {
            let binding = resources
                .iter()
                .position(|&resource| resource == index)
                .map_or(64 + index, |binding| binding as u32);
            writeln!(
                bindings,
                "public static const let B{index}: uint = {binding};"
            )
            .expect("向 String 写入不会失败");
        }
        fs::write(directory.join("atmosphere_bindings.slang"), bindings)?;
        sources.push(format!("../../shaders/atmosphere/{source}.slang"));
        outputs.push(format!("{entry}.spv"));
        arguments.push(["-I".to_owned(), directory.to_string_lossy().into_owned()]);
    }
    let argument_refs: Vec<_> = arguments
        .iter()
        .map(|arguments| [arguments[0].as_str(), arguments[1].as_str()])
        .collect();
    let mut recipes: Vec<_> = stages
        .iter()
        .enumerate()
        .map(|(index, (_, entry, _))| Recipe {
            source: &sources[index],
            entry,
            stage: "compute",
            output: &outputs[index],
            target: Target::Spirv("spirv_1_3"),
            args: &argument_refs[index],
        })
        .collect();
    if cfg!(feature = "native") {
        for (source, entry, output) in [
            (
                "../../shaders/atmosphere/shadow_prepare.slang",
                "shadow_prepare",
                "shadow_prepare.spv",
            ),
            (
                "../../shaders/atmosphere/shadow_demand.slang",
                "shadow_demand",
                "shadow_demand.spv",
            ),
        ] {
            recipes.push(Recipe {
                source,
                entry,
                stage: "compute",
                output,
                target: Target::Spirv("spirv_1_3"),
                args: &[],
            });
        }
    }
    build(&recipes, &["../../shaders"], &["src", "data"])?;
    Ok(())
}
