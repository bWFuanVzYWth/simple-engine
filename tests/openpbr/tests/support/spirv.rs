//! 最终SPIR-V的可达调用、资源与控制图读取；物理断言留在专项测试中。
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Function {
    calls: Vec<u32>,
    loads: Vec<u32>,
    image_operations: usize,
}

pub(super) fn energy_uses(bytes: &[u8]) -> (BTreeSet<u32>, usize) {
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

#[derive(Default)]
pub(super) struct Audit {
    pub(super) operations: BTreeMap<u32, usize>,
    pub(super) extended: BTreeMap<u32, usize>,
    pub(super) input_components: BTreeSet<Vec<Option<u32>>>,
    pub(super) constant_branches: usize,
    pub(super) runtime_switches: usize,
}

pub(super) fn executable_audit(bytes: &[u8]) -> Audit {
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
