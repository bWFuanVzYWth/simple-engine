//! cornell_box 的 Slang 生产入口构建；产物由 Cargo OUT_DIR 管理。
use simple_engine::shader_build::{self, Recipe, Target, build};
use simple_engine::shader_library;

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
    build(&recipes, &[shader_library::root()], &[])
}
