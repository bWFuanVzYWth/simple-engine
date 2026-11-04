//! 检查最终 SPIR-V 的实际能量资源使用，不搜索 debug 中嵌入的源码。
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Function {
    calls: Vec<u32>,
    loads: Vec<u32>,
    image_operations: usize,
}

fn energy_uses(bytes: &[u8]) -> (BTreeSet<u32>, usize) {
    assert_eq!(bytes.len() % 4, 0);
    let words: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    assert_eq!(words[0], 0x0723_0203);
    let mut entry = None;
    let mut bindings = BTreeMap::<u32, (Option<u32>, Option<u32>)>::new();
    let mut uniform_constants = BTreeSet::new();
    let mut functions = BTreeMap::<u32, Function>::new();
    let mut active = None;
    let mut cursor = 5;
    while cursor < words.len() {
        let count = (words[cursor] >> 16) as usize;
        let opcode = words[cursor] & 65535;
        assert!(count > 0 && count <= words.len() - cursor);
        let operands = &words[cursor + 1..cursor + count];
        match opcode {
            15 => {
                assert!(
                    entry.replace(operands[1]).is_none(),
                    "fixture只有一个entrypoint"
                );
            }
            54 => {
                active = Some(operands[1]);
                functions.entry(operands[1]).or_default();
            }
            59 if operands[2] == 0 => {
                uniform_constants.insert(operands[1]);
            }
            71 if operands[1] == 33 => {
                bindings.entry(operands[0]).or_default().1 = Some(operands[2])
            }
            71 if operands[1] == 34 => {
                bindings.entry(operands[0]).or_default().0 = Some(operands[2])
            }
            _ => {}
        }
        if let Some(function) = active {
            let function = functions.get_mut(&function).unwrap();
            match opcode {
                57 => function.calls.push(operands[2]),
                61 => function.loads.push(operands[2]),
                87..=98 => function.image_operations += 1,
                _ => {}
            }
        }
        if opcode == 56 {
            active = None;
        }
        cursor += count;
    }
    let mut pending = vec![entry.unwrap()];
    let mut visited = BTreeSet::new();
    let mut energy_bindings = BTreeSet::new();
    let mut image_operations = 0;
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        let function = &functions[&id];
        pending.extend(&function.calls);
        image_operations += function.image_operations;
        for &pointer in &function.loads {
            if uniform_constants.contains(&pointer)
                && let Some(&(Some(0), Some(binding @ (2 | 3)))) = bindings.get(&pointer)
            {
                energy_bindings.insert(binding);
            }
        }
    }
    (energy_bindings, image_operations)
}

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

#[derive(Default)]
struct Audit {
    operations: BTreeMap<u32, usize>,
    extended: BTreeMap<u32, usize>,
    input_components: BTreeSet<Vec<Option<u32>>>,
    constant_branches: usize,
    runtime_switches: usize,
}

fn executable_audit(bytes: &[u8]) -> Audit {
    let words: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|word| u32::from_le_bytes(*word))
        .collect();
    let mut functions = BTreeMap::<u32, Vec<(u32, Vec<u32>)>>::new();
    let mut imports = BTreeMap::<u32, String>::new();
    let mut constants = BTreeMap::<u32, u32>::new();
    let mut known = BTreeSet::new();
    let mut input = None;
    let mut entry = None;
    let mut active = None;
    let mut cursor = 5;
    while cursor < words.len() {
        let count = (words[cursor] >> 16) as usize;
        let op = words[cursor] & 65535;
        let a = &words[cursor + 1..cursor + count];
        match op {
            11 => {
                let text: Vec<_> = a[1..]
                    .iter()
                    .flat_map(|value| value.to_le_bytes())
                    .take_while(|value| *value != 0)
                    .collect();
                imports.insert(a[0], String::from_utf8(text).unwrap());
            }
            15 => entry = Some(a[1]),
            41 | 42 => {
                known.insert(a[1]);
            }
            43 => {
                constants.insert(a[1], a[2]);
                known.insert(a[1]);
            }
            44 if a[2..].iter().all(|input| known.contains(input)) => {
                known.insert(a[1]);
            }

            54 => active = Some(a[1]),
            71 if a[1] == 33 && a[2] == 0 => input = Some(a[0]),
            _ => {}
        }
        if let Some(id) = active {
            functions.entry(id).or_default().push((op, a.to_vec()));
        }
        if op == 56 {
            active = None;
        }
        cursor += count;
    }
    let mut pending = vec![entry.unwrap()];
    let mut visited = BTreeSet::new();
    let mut audit = Audit::default();
    let mut pointers = BTreeMap::from([(input.unwrap(), Vec::<Option<u32>>::new())]);
    let mut values = BTreeMap::<u32, Vec<Option<u32>>>::new();
    while let Some(id) = pending.pop() {
        if !visited.insert(id) {
            continue;
        }
        for (op, a) in &functions[&id] {
            if *op == 12 {
                if imports[&a[2]].starts_with("NonSemantic.") {
                    continue;
                }
                assert_eq!(imports[&a[2]], "GLSL.std.450");
                *audit.extended.entry(a[3]).or_default() += 1;
            }
            *audit.operations.entry(*op).or_default() += 1;
            let constant_inputs = match (*op, a.as_slice()) {
                (44 | 80, [_, result, inputs @ ..]) => Some((*result, inputs)),
                (79 | 82, [_, result, _, _, _, ..]) => Some((*result, &a[2..4])),
                (81, [_, result, input, _, ..]) => Some((*result, std::slice::from_ref(input))),
                (83 | 109..=116 | 124 | 126..=127 | 154..=157 | 168 | 200, [_, result, input]) => {
                    Some((*result, std::slice::from_ref(input)))
                }
                (77 | 128..=141 | 164..=167 | 170..=191 | 194..=199, [_, result, _, _]) => {
                    Some((*result, &a[2..]))
                }
                (78 | 169, [_, result, _, _, _]) => Some((*result, &a[2..])),
                (12, [_, result, set, operation, inputs @ ..])
                    if imports[set] == "GLSL.std.450"
                        && (matches!(*operation, 37..=42) && inputs.len() == 2
                            || matches!(*operation, 43..=45) && inputs.len() == 3) =>
                {
                    Some((*result, inputs))
                }
                _ => None,
            };
            if let Some((result, inputs)) = constant_inputs
                && inputs.iter().all(|input| known.contains(input))
            {
                known.insert(result);
            }
            match *op {
                57 => pending.push(a[2]),
                65 | 66 => {
                    if let Some(base) = pointers.get(&a[2]) {
                        let mut path = base.clone();
                        path.extend(a[3..].iter().map(|id| constants.get(id).copied()));
                        pointers.insert(a[1], path);
                    }
                }
                61 => {
                    if let Some(path) = pointers.get(&a[2]) {
                        values.insert(a[1], path.clone());
                        if path.len() >= 5 {
                            audit.input_components.insert(path.clone());
                        }
                    }
                }
                81 => {
                    if let Some(base) = values.get(&a[2]) {
                        let mut path = base.clone();
                        path.extend(a[3..].iter().copied().map(Some));
                        audit.input_components.insert(path.clone());
                        values.insert(a[1], path);
                    }
                }
                79 => {
                    for &component in &a[4..] {
                        let source = if component < 4 { a[2] } else { a[3] };
                        if let Some(base) = values.get(&source) {
                            let mut path = base.clone();
                            path.push(Some(component % 4));
                            audit.input_components.insert(path);
                        }
                    }
                }
                250 if known.contains(&a[0]) => audit.constant_branches += 1,
                251 if a.len() > 2 => {
                    audit.runtime_switches += 1;
                    audit.constant_branches += usize::from(known.contains(&a[0]));
                }
                _ => {}
            }
        }
    }
    audit
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
