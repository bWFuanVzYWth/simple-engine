//! wgpu_vulkan_tests 的 Slang 生产入口构建；产物由 Cargo OUT_DIR 管理。
use simple_engine::shader_build::{self, Recipe, Target, build};

fn main() -> Result<(), shader_build::Error> {
    let mut recipes = [
        (
            "shaders/prepare.slang",
            "prepare_main",
            "compute",
            "prepare_main.spv",
        ),
        (
            "shaders/readback.slang",
            "readback_main",
            "compute",
            "readback_main.spv",
        ),
        (
            "shaders/trace.slang",
            "raygen_main",
            "raygeneration",
            "raygen_main.spv",
        ),
        ("shaders/trace.slang", "miss_main", "miss", "miss_main.spv"),
        (
            "shaders/trace.slang",
            "closest_hit_main",
            "closesthit",
            "closest_hit_main.spv",
        ),
        (
            "shaders/callable.slang",
            "callable_raygen",
            "raygeneration",
            "callable_raygen.spv",
        ),
        (
            "shaders/callable.slang",
            "callable_miss",
            "miss",
            "callable_miss.spv",
        ),
        (
            "shaders/callable.slang",
            "callable_closest",
            "closesthit",
            "callable_closest.spv",
        ),
        (
            "shaders/callable.slang",
            "callable_add",
            "callable",
            "callable_add.spv",
        ),
        (
            "shaders/callable.slang",
            "callable_scale",
            "callable",
            "callable_scale.spv",
        ),
        (
            "shaders/instances.slang",
            "instance_raygen",
            "raygeneration",
            "instance_raygen.spv",
        ),
        (
            "shaders/instances.slang",
            "instance_miss",
            "miss",
            "instance_miss.spv",
        ),
        (
            "shaders/instances.slang",
            "instance_closest",
            "closesthit",
            "instance_closest.spv",
        ),
        (
            "shaders/present.slang",
            "vertex_main",
            "vertex",
            "vertex_main.spv",
        ),
        (
            "shaders/present.slang",
            "fragment_main",
            "fragment",
            "fragment_main.spv",
        ),
    ]
    .map(|(source, entry, stage, output)| Recipe {
        source,
        entry,
        stage,
        output,
        target: Target::Spirv("spirv_1_4"),
        args: &[],
    })
    .into_iter()
    .collect::<Vec<_>>();
    recipes.extend(
        [
            ("hit_groups_raygen", "raygeneration"),
            ("hit_groups_miss", "miss"),
            ("hit_groups_first", "closesthit"),
            ("hit_groups_second", "closesthit"),
            ("hit_groups_third", "closesthit"),
            ("hit_groups_add", "callable"),
            ("hit_groups_scale", "callable"),
        ]
        .into_iter()
        .map(|(entry, stage)| Recipe {
            source: "shaders/hit_groups.slang",
            entry,
            stage,
            output: match entry {
                "hit_groups_raygen" => "hit_groups_raygen.spv",
                "hit_groups_miss" => "hit_groups_miss.spv",
                "hit_groups_first" => "hit_groups_first.spv",
                "hit_groups_second" => "hit_groups_second.spv",
                "hit_groups_third" => "hit_groups_third.spv",
                "hit_groups_add" => "hit_groups_add.spv",
                _ => "hit_groups_scale.spv",
            },
            target: Target::Spirv("spirv_1_4"),
            args: &["-capability", "spvShaderInvocationReorderEXT"],
        }),
    );
    recipes.extend([
        Recipe {
            source: "shaders/sampled.slang",
            entry: "sampled_prepare",
            stage: "compute",
            output: "sampled_prepare.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &["-DSAMPLED_PREPARE=1"],
        },
        Recipe {
            source: "shaders/sampled.slang",
            entry: "sampled_raygen",
            stage: "raygeneration",
            output: "sampled_raygen.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &[],
        },
        Recipe {
            source: "shaders/sampled.slang",
            entry: "sampled_miss",
            stage: "miss",
            output: "sampled_miss.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &[],
        },
        Recipe {
            source: "shaders/sampled.slang",
            entry: "sampled_closest",
            stage: "closesthit",
            output: "sampled_closest.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &[],
        },
    ]);
    recipes.extend(
        [
            ("native_raygen", "raygeneration", "native_raygen.spv"),
            ("native_miss", "miss", "native_miss.spv"),
            ("native_closest", "closesthit", "native_closest.spv"),
            ("native_any_hit", "anyhit", "native_any_hit.spv"),
            ("native_post", "compute", "native_post.spv"),
        ]
        .map(|(entry, stage, output)| Recipe {
            source: "shaders/native.slang",
            entry,
            stage,
            output,
            target: Target::Spirv("spirv_1_4"),
            args: &[],
        }),
    );
    recipes.push(Recipe {
        source: "shaders/uniform.slang",
        entry: "uniform_main",
        stage: "compute",
        output: "uniform_main.spv",
        target: Target::Spirv("spirv_1_4"),
        args: &[],
    });
    recipes.extend(
        [
            ("reorder_raygen", "raygeneration", "reorder_raygen.spv"),
            ("reorder_miss", "miss", "reorder_miss.spv"),
            ("reorder_closest", "closesthit", "reorder_closest.spv"),
        ]
        .map(|(entry, stage, output)| Recipe {
            source: "shaders/reorder.slang",
            entry,
            stage,
            output,
            target: Target::Spirv("spirv_1_4"),
            args: &["-capability", "spvShaderInvocationReorderEXT"],
        }),
    );
    build(&recipes, &[], &[])
}
