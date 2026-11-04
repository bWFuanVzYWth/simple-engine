//! hello_triangle 的 Slang 生产入口构建；产物由 Cargo OUT_DIR 管理。
use simple_engine::shader_build::{self, Recipe, Target, build};

fn main() -> Result<(), shader_build::Error> {
    let recipes = [
        ("vertex_main", "vertex", "triangle.vert.spv"),
        ("fragment_main", "fragment", "triangle.frag.spv"),
    ]
    .map(|(entry, stage, output)| Recipe {
        source: "shaders/triangle.slang",
        entry,
        stage,
        output,
        target: Target::Spirv("spirv_1_3"),
        args: &[],
    });
    build(&recipes, &[], &[])
}
