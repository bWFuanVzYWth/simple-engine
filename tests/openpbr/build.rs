//! 构建 OpenPBR 生产、特化和输运测试程序。
use simple_engine::{
    shader_build::{self, Recipe, Target},
    shader_library,
};

fn main() -> Result<(), shader_build::Error> {
    let mut recipes = vec![
        Recipe {
            source: "shaders/numerical.slang",
            entry: "main",
            stage: "compute",
            output: "numerical.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &["-fp-mode", "precise"],
        },
        Recipe {
            source: "shaders/production.slang",
            entry: "main",
            stage: "compute",
            output: "production.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &["-fp-mode", "precise"],
        },
        Recipe {
            source: "shaders/transport.slang",
            entry: "main",
            stage: "compute",
            output: "transport.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &["-fp-mode", "precise"],
        },
        Recipe {
            source: "shaders/transport.slang",
            entry: "vectors",
            stage: "compute",
            output: "transport-vectors.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &["-fp-mode", "precise"],
        },
    ];
    const SPECIALIZATIONS: [(&str, &[&str]); 8] = [
        (
            "specialized-opaque.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=0"],
        ),
        (
            "specialized-subsurface.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=1"],
        ),
        (
            "specialized-conductor.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=2"],
        ),
        (
            "specialized-solid.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=3"],
        ),
        (
            "specialized-thin.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=4"],
        ),
        (
            "specialized-delta-conductor.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=5"],
        ),
        (
            "specialized-delta-solid.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=6"],
        ),
        (
            "specialized-delta-thin.spv",
            &["-fp-mode", "precise", "-DOPENPBR_CAPABILITY=7"],
        ),
    ];
    recipes.extend(SPECIALIZATIONS.into_iter().map(|(output, args)| Recipe {
        source: "shaders/specialized.slang",
        entry: "main",
        stage: "compute",
        output,
        target: Target::Spirv("spirv_1_4"),
        args,
    }));
    const FLOWS: [(&str, &[&str]); 30] = [
        (
            "flow-opaque.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=195"],
        ),
        (
            "flow-subsurface.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=203"],
        ),
        (
            "flow-conductor.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=196"],
        ),
        (
            "flow-solid.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=978"],
        ),
        (
            "flow-thin.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=994"],
        ),
        (
            "flow-delta-conductor.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=68"],
        ),
        (
            "flow-delta-solid.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=338"],
        ),
        (
            "flow-delta-thin.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=354"],
        ),
        (
            "flow-opaque-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=131"],
        ),
        (
            "flow-subsurface-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=139"],
        ),
        (
            "flow-conductor-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=132"],
        ),
        (
            "flow-solid-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=658"],
        ),
        (
            "flow-thin-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=674"],
        ),
        (
            "flow-opaque-smooth.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=67"],
        ),
        (
            "flow-mixed-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2183"],
        ),
        (
            "flow-mixed-smooth.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2119"],
        ),
        (
            "flow-mixed-subsurface-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2191"],
        ),
        (
            "flow-mixed-subsurface.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2255"],
        ),
        (
            "flow-subsurface-only.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=138"],
        ),
        (
            "flow-absorber.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=0"],
        ),
        (
            "flow-lambert.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=1"],
        ),
        (
            "flow-subsurface-matched.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=8"],
        ),
        (
            "flow-solid-matched.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=272"],
        ),
        (
            "flow-thin-matched.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=288"],
        ),
        (
            "flow-black-smooth.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=66"],
        ),
        (
            "flow-solid-rough-matched.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=914"],
        ),
        (
            "flow-metal-null-continuous.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2180"],
        ),
        (
            "flow-metal-null-smooth.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2116"],
        ),
        (
            "flow-metal-lambert.spv",
            &["-fp-mode", "precise", "-DOPENPBR_FLOW=2181"],
        ),
        (
            "flow-binary-metal-sss.spv",
            &[
                "-fp-mode",
                "precise",
                "-DOPENPBR_FLOW=2191",
                "-DOPENPBR_VARIANT=6144",
                "-DOPENPBR_RECIPE_VARIANT=1",
            ],
        ),
    ];
    recipes.extend(FLOWS.into_iter().map(|(output, args)| Recipe {
        source: "shaders/flow.slang",
        entry: "main",
        stage: "compute",
        output,
        target: Target::Spirv("spirv_1_4"),
        args,
    }));
    const SHARING: [(&str, &[&str]); 4] = [
        (
            "flow-shared-opaque.spv",
            &[
                "-fp-mode",
                "precise",
                "-DOPENPBR_FLOW=131",
                "-DOPENPBR_SHARED=1",
            ],
        ),
        (
            "flow-separate-opaque.spv",
            &[
                "-fp-mode",
                "precise",
                "-DOPENPBR_FLOW=131",
                "-DOPENPBR_SHARED=0",
            ],
        ),
        (
            "flow-shared-solid.spv",
            &[
                "-fp-mode",
                "precise",
                "-DOPENPBR_FLOW=658",
                "-DOPENPBR_SHARED=1",
            ],
        ),
        (
            "flow-separate-solid.spv",
            &[
                "-fp-mode",
                "precise",
                "-DOPENPBR_FLOW=658",
                "-DOPENPBR_SHARED=0",
            ],
        ),
    ];
    recipes.extend(SHARING.into_iter().map(|(output, args)| Recipe {
        source: "shaders/flow_shared.slang",
        entry: "main",
        stage: "compute",
        output,
        target: Target::Spirv("spirv_1_4"),
        args,
    }));
    shader_build::build(
        &recipes,
        &[shader_library::root()],
        &[
            "src",
            "tests",
            "../../crates/openpbr",
            "../../shaders/bsdf/openpbr",
        ],
    )
}
