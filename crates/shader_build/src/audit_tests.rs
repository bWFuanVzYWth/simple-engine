use super::*;

fn instruction(opcode: u32, arguments: &[u32]) -> Vec<u32> {
    let mut words = vec![((arguments.len() as u32 + 1) << 16) | opcode];
    words.extend_from_slice(arguments);
    words
}

fn import(name: &str) -> Vec<u32> {
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    while !bytes.len().is_multiple_of(4) {
        bytes.push(0);
    }
    let mut args = vec![7];
    args.extend(
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|word| u32::from_le_bytes(*word)),
    );
    instruction(11, &args)
}

fn module(extra: &[u32]) -> Vec<u8> {
    let mut words = vec![0x0723_0203, 0x0001_0300, 0, 100, 0];
    words.extend(instruction(21, &[1, 32, 0]));
    words.extend(instruction(43, &[1, 2, 17]));
    words.extend(instruction(1, &[1, 3]));
    words.extend_from_slice(extra);
    words.extend(instruction(54, &[1, 4, 0, 5]));
    words.extend(instruction(128, &[1, 6, 2, 2]));
    words.extend(instruction(56, &[]));
    words.into_iter().flat_map(u32::to_le_bytes).collect()
}

#[test]
fn pairing_excludes_only_explicit_debug_records() {
    let mut debug = import(DEBUG_SET);
    debug.extend(instruction(12, &[1, 8, 7, 1, 2]));
    debug.extend(instruction(5, &[4, 0]));
    verify_audit_pair(&module(&[]), &module(&debug)).unwrap();
}

#[test]
fn pairing_rejects_core_arithmetic_changes() {
    let main = module(&[]);
    let mut changed = main.clone();
    let offset = changed.len() - 6 * 4;
    changed[offset..offset + 4].copy_from_slice(&((5u32 << 16) | 130).to_le_bytes());
    assert!(verify_audit_pair(&main, &changed).is_err());
}

#[test]
fn pairing_keeps_unused_types_constants_and_undef() {
    for extra in [
        instruction(22, &[9, 32]),
        instruction(43, &[1, 9, 99]),
        instruction(1, &[1, 9]),
    ] {
        assert!(verify_audit_pair(&module(&[]), &module(&extra)).is_err());
    }
}

#[test]
fn pairing_rejects_a_changed_header_or_signedness() {
    let main = module(&[]);
    for offset in [12, 32] {
        let mut other = main.clone();
        other[offset] ^= 1;
        assert!(verify_audit_pair(&main, &other).is_err());
    }
}

#[test]
fn pairing_does_not_ignore_debug_printf() {
    let data = module(&import("NonSemantic.DebugPrintf"));
    assert!(matches!(
        verify_audit_pair(&data, &data),
        Err(Error::AuditPair(_))
    ));
}

#[test]
fn pairing_rejects_truncated_or_zero_length_instructions() {
    let main = module(&[]);
    for bytes in [
        main[..19].to_vec(),
        main[..main.len() - 1].to_vec(),
        [main.as_slice(), &[0, 0, 0, 0]].concat(),
    ] {
        assert!(verify_audit_pair(&main, &bytes).is_err());
    }
}
