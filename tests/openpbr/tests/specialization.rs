//! 检查最终 SPIR-V 的实际能量资源使用，不搜索 debug 中嵌入的源码。
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
#[path = "support/spirv.rs"]
mod spirv;
use spirv::{energy_uses, executable_audit};

#[test]
fn compiled_profiles_keep_only_required_energy_resources() {
    let shaders: [(&str, &[u8], bool); 8] = [
        (
            "opaque",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-opaque.spv")),
            true,
        ),
        (
            "SSS",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-subsurface.spv")),
            true,
        ),
        (
            "conductor",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-conductor.spv")),
            false,
        ),
        (
            "solid",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-solid.spv")),
            true,
        ),
        (
            "thin",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-thin.spv")),
            false,
        ),
        (
            "delta conductor",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-delta-conductor.spv")),
            false,
        ),
        (
            "delta solid",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-delta-solid.spv")),
            false,
        ),
        (
            "delta thin",
            include_bytes!(concat!(env!("OUT_DIR"), "/specialized-delta-thin.spv")),
            false,
        ),
    ];
    for (label, shader, uses_energy) in shaders {
        let (bindings, operations) = energy_uses(shader);
        if uses_energy {
            assert_eq!(
                bindings,
                BTreeSet::from([2, 3]),
                "{label}必须使用真实纹理与sampler"
            );
            assert!(operations > 0, "{label}缺少真实能量读取");
        } else {
            assert!(bindings.is_empty(), "{label}保留了不需要的能量资源使用");
            assert_eq!(operations, 0, "{label}不应采样能量纹理");
        }
    }
}

#[test]
fn flow_programs_remove_excluded_control_and_resource_uses() {
    let profiles: [(&str, &[u8], u32); 29] = [
        (
            "opaque",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-opaque.spv")),
            195,
        ),
        (
            "subsurface",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface.spv")),
            203,
        ),
        (
            "conductor",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-conductor.spv")),
            196,
        ),
        (
            "solid",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid.spv")),
            978,
        ),
        (
            "thin",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-thin.spv")),
            994,
        ),
        (
            "delta-conductor",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-delta-conductor.spv")),
            68,
        ),
        (
            "delta-solid",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-delta-solid.spv")),
            338,
        ),
        (
            "delta-thin",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-delta-thin.spv")),
            354,
        ),
        (
            "opaque-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-opaque-continuous.spv")),
            131,
        ),
        (
            "subsurface-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface-continuous.spv")),
            139,
        ),
        (
            "conductor-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-conductor-continuous.spv")),
            132,
        ),
        (
            "solid-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid-continuous.spv")),
            658,
        ),
        (
            "thin-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-thin-continuous.spv")),
            674,
        ),
        (
            "opaque-smooth",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-opaque-smooth.spv")),
            67,
        ),
        (
            "mixed-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-mixed-continuous.spv")),
            2183,
        ),
        (
            "mixed-smooth",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-mixed-smooth.spv")),
            2119,
        ),
        (
            "mixed-subsurface-continuous",
            include_bytes!(concat!(
                env!("OUT_DIR"),
                "/flow-mixed-subsurface-continuous.spv"
            )),
            2191,
        ),
        (
            "mixed-subsurface",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-mixed-subsurface.spv")),
            2255,
        ),
        (
            "subsurface-only",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface-only.spv")),
            138,
        ),
        (
            "absorber",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-absorber.spv")),
            0,
        ),
        (
            "lambert",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-lambert.spv")),
            1,
        ),
        (
            "subsurface-matched",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-subsurface-matched.spv")),
            8,
        ),
        (
            "solid-matched",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid-matched.spv")),
            272,
        ),
        (
            "thin-matched",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-thin-matched.spv")),
            288,
        ),
        (
            "black-smooth",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-black-smooth.spv")),
            66,
        ),
        (
            "solid-rough-matched",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-solid-rough-matched.spv")),
            914,
        ),
        (
            "metal-null-continuous",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-metal-null-continuous.spv")),
            2180,
        ),
        (
            "metal-null-smooth",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-metal-null-smooth.spv")),
            2116,
        ),
        (
            "metal-lambert",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-metal-lambert.spv")),
            2181,
        ),
    ];
    let mut dead_branches = Vec::new();
    for (label, shader, flow) in profiles {
        let dielectric_reflection = flow & 2 != 0;
        let subsurface = flow & 8 != 0;
        let solid = flow & 16 != 0;
        let thin = flow & 32 != 0;
        let continuous = flow & (1 | 8 | 128 | 512) != 0;
        let energy = dielectric_reflection && !thin && continuous;
        let (bindings, operations) = energy_uses(shader);
        assert_eq!(!bindings.is_empty(), energy, "{label} energy resources");
        assert_eq!(operations > 0, energy, "{label} energy operations");
        let audit = executable_audit(shader);
        if audit.constant_branches != 0 {
            dead_branches.push((label, audit.constant_branches));
        }
        assert_eq!(
            audit.runtime_switches, 0,
            "{label} retains runtime capability dispatch"
        );
        let excluded_surface = [
            !dielectric_reflection || solid || thin,
            flow & 2048 == 0,
            !subsurface,
            !thin,
        ];
        for (component, excluded) in excluded_surface.into_iter().enumerate() {
            let path = vec![Some(0), None, Some(0), Some(1), Some(component as u32)];
            assert!(
                !excluded || !audit.input_components.contains(&path),
                "{label} still consumes excluded surface[{component}]"
            );
        }
        for component in 0..4 {
            let path = vec![Some(0), None, Some(0), Some(2), Some(component)];
            assert!(
                !audit.input_components.contains(&path),
                "{label} still reads runtime material classification"
            );
        }
        if flow & (128 | 512) == 0 {
            let roughness = vec![Some(0), None, Some(0), Some(0), Some(3)];
            assert!(
                !audit.input_components.contains(&roughness),
                "{label} still reads roughness with only delta specular support"
            );
            assert_eq!(
                audit.extended.get(&29),
                None,
                "{label} retains unused MS.x/y Exp2"
            );
        }
        if matches!(label, "lambert" | "solid-matched" | "absorber") {
            for op in [26, 27, 28, 29, 30] {
                assert_eq!(
                    audit.extended.get(&op),
                    None,
                    "{label} retains nonlinear Fresnel/energy operations {op}"
                );
            }
        }
    }
    assert!(
        dead_branches.is_empty(),
        "statically dead branches: {dead_branches:?}"
    );
}

#[test]
fn combined_vertex_prepares_the_energy_lookup_once() {
    let profiles = [
        (
            "opaque",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-shared-opaque.spv")).as_slice(),
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-separate-opaque.spv")).as_slice(),
        ),
        (
            "solid",
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-shared-solid.spv")).as_slice(),
            include_bytes!(concat!(env!("OUT_DIR"), "/flow-separate-solid.spv")).as_slice(),
        ),
    ];
    for (label, shared, separate) in profiles {
        let (_, shared_images) = energy_uses(shared);
        let (_, separate_images) = energy_uses(separate);
        assert_eq!(
            shared_images, 1,
            "{label} combined vertex must prepare the LUT exactly once"
        );
        assert!(
            separate_images > shared_images,
            "{label} fixture must expose repeated independent preparation"
        );
    }
}
