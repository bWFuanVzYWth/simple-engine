//! 圆孔标定与原生 Slang 程序的离线构建入口。
mod airy_weights;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use shader_build::{Recipe, Target};
    use std::{fmt::Write, path::PathBuf};
    if std::env::var("CARGO_CFG_TARGET_ARCH")? == "wasm32" {
        return Ok(());
    }
    let out =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or(shader_build::Error::OutputDirectory)?);
    let weights = airy_weights::calculate();
    let mut module = String::from("module circular_weights;\nnamespace circular_weights {\n");
    writeln!(
        module,
        "public static const uint levels = {};",
        weights.wings.len()
    )?;
    let taps = airy_weights::BINOMIAL;
    let mut quarter = [0.; 16];
    for (x, a) in taps.into_iter().enumerate() {
        for (y, b) in taps.into_iter().enumerate() {
            quarter[x + 2 * y] += a * b;
        }
    }
    for (name, values) in [
        ("binomial", taps.as_slice()),
        ("quarter_taps", quarter.as_slice()),
    ] {
        writeln!(
            module,
            "public static const float {name}[{}] = {{",
            values.len()
        )?;
        for &v in values {
            writeln!(module, "{v:?},")?;
        }
        module.push_str("};\n");
    }
    for (name, values) in [
        ("core", weights.core.as_slice()),
        ("wings", weights.wings.as_slice()),
    ] {
        writeln!(
            module,
            "public static const float3 {name}[{}] = {{",
            values.len()
        )?;
        for [r, g, b] in values {
            writeln!(module, "float3({r:?}, {g:?}, {b:?}),")?;
        }
        module.push_str("};\n");
    }
    module.push_str("}\n");
    std::fs::write(out.join("circular_weights.slang"), module)?;
    std::fs::write(
        out.join("levels.rs"),
        format!("const LEVELS: usize = {};", airy_weights::LEVELS),
    )?;
    let include = out.to_string_lossy();
    let args = ["-I", &include];
    let recipes = [
        ("circular_quarter", "circular_quarter.spv"),
        ("circular_down", "circular_down.spv"),
        ("circular_up", "circular_up.spv"),
        ("circular_up_pair", "circular_up_pair.spv"),
        ("circular_composite", "circular_composite.spv"),
    ]
    .map(|(entry, output)| Recipe {
        source: "shaders/circular.slang",
        entry,
        output,
        stage: "compute",
        target: Target::Spirv("spirv_1_4"),
        args: &args,
    });
    shader_build::build(&recipes, &["shaders"], &["src", "airy_weights.rs"])?;
    Ok(())
}
