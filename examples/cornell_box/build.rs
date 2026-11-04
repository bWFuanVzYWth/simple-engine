//! cornell_box 的 Slang 生产入口构建；产物由 Cargo OUT_DIR 管理。
use simple_engine::shader_build::{self, Recipe, Target, build};
use simple_engine::shader_library;
use std::{fs, path::Path};

fn main() -> Result<(), shader_build::Error> {
    let recipes = [
        ("render.spv", "render_main", "raygeneration"),
        ("miss.spv", "miss_main", "miss"),
        ("closest_hit.spv", "closest_hit_main", "closesthit"),
    ]
    .map(|(output, entry, stage)| Recipe {
        source: "shaders/render.slang",
        entry,
        stage,
        output,
        target: Target::Spirv("spirv_1_4"),
        args: &["-fp-mode", "precise"],
    });
    build(&recipes, &[shader_library::root()], &[])?;
    let root = Path::new(shader_library::root());
    let mut hash = blake3::Hasher::new();
    for path in [
        Path::new("shaders/render.slang").to_owned(),
        root.join("pt/sampling.slang"),
        root.join("pt/bsdf.slang"),
        root.join("pt/mis.slang"),
        root.join("geometry/ray.slang"),
    ] {
        hash.update(&fs::read(&path).map_err(|source| shader_build::Error::Io { path, source })?);
    }
    println!(
        "cargo::rustc-env=CORNELL_SHADER_BLAKE3={}",
        hash.finalize().to_hex()
    );
    Ok(())
}
