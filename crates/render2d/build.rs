//! 由同一 Slang 源码构建 native SPIR-V 与 Web WGSL，并核对四份反射后导出 ABI 常量。

#[cfg(feature = "gpu")]
mod build_abi;

#[cfg(not(feature = "gpu"))]
fn main() {}

#[cfg(feature = "gpu")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use shader_build::{Recipe, Target, build};
    use std::{env, path::PathBuf};

    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo 未设置 OUT_DIR")?);
    let stages = [
        (
            "vertex_main",
            "vertex",
            "render2d.vert.spv",
            Target::Spirv("spirv_1_3"),
        ),
        (
            "fragment_main",
            "fragment",
            "render2d.frag.spv",
            Target::Spirv("spirv_1_3"),
        ),
        ("vertex_main", "vertex", "render2d.vert.wgsl", Target::Wgsl),
        (
            "fragment_main",
            "fragment",
            "render2d.frag.wgsl",
            Target::Wgsl,
        ),
    ];
    let reflections: Vec<_> = stages
        .iter()
        .map(|(_, _, file, _)| {
            output
                .join(format!("{file}.json"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let arguments: Vec<_> = reflections
        .iter()
        .map(|path| ["-reflection-json", path.as_str()])
        .collect();
    let recipes: Vec<_> = stages
        .iter()
        .enumerate()
        .map(|(index, (entry, stage, output, target))| Recipe {
            source: "shaders/render2d.slang",
            entry,
            stage,
            output,
            target: *target,
            args: &arguments[index],
        })
        .collect();
    build(&recipes, &[], &["src", "build_abi.rs"])?;
    build_abi::verify(&output)?;
    Ok(())
}
