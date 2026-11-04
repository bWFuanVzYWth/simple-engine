use crate::api::{self, Dag, DenseRegion, Path};

pub fn path(code: u64) -> Path {
    Path::from_value(code).unwrap()
}

pub fn scene_value(scene: &str, code: u64, lod: u8) -> u32 {
    let (x, y, z) = path(code).decode();
    let side = 1_u32 << lod;
    match scene {
        "empty" => 0,
        "solid" => 7,
        "terrain" => u32::from(y < side * 3 / 8 + (x * 3 + z * 5) % (side / 4 + 1)) * 7,
        "checker" => ((x ^ y ^ z) & 1) * 7,
        "repeated" => (code as u32 & 63) * 7 + ((code as u32 >> 6) & 1) * 101 + 1,
        "noise" => (code as u32).wrapping_mul(0x9e37_79b9).rotate_left(13) ^ 0xd1b5_4a32,
        _ => panic!("未知场景：{scene}"),
    }
}

pub fn scene_values(scene: &str, lod: u8) -> Vec<u32> {
    (0..1_u64 << (3 * lod))
        .map(|code| scene_value(scene, code, lod))
        .collect()
}

pub fn build(values: &[u32], lod: u8) -> Dag {
    let mut dag = api::new();
    let root = api::root(&dag);
    api::set_dense_regions(
        &mut dag,
        root,
        &[DenseRegion {
            path: path(0),
            lod,
            values,
        }],
    );
    dag
}

pub fn build_scene(scene: &str, lod: u8) -> Dag {
    build(&scene_values(scene, lod), lod)
}

pub fn paths(pattern: &str, count: usize, lod: u8) -> Vec<Path> {
    let mask = (1_u64 << (3 * lod)) - 1;
    let mut codes: Vec<_> = (0..count as u64)
        .map(|index| match pattern {
            "cluster" => index & mask,
            "scatter" => index.wrapping_mul(0x9e37_79b9) & mask,
            "repeated" => (index / 16 * 31) & mask,
            _ => panic!("未知路径分布：{pattern}"),
        })
        .collect();
    codes.sort_unstable();
    codes.into_iter().map(path).collect()
}

pub fn digest(values: &[u32]) -> u64 {
    values.iter().fold(0xcbf2_9ce4_8422_2325, |hash, value| {
        (hash ^ u64::from(*value)).wrapping_mul(0x100_0000_01b3)
    })
}

pub fn emit(name: &str, digest: u64) {
    println!("SVDAG_ORACLE\t{name}\t{digest:016x}");
}

pub fn emit_after(name: &str, digest: u64) {
    println!("SVDAG_AFTER\t{name}\t{digest:016x}");
}

pub fn work(name: &str, elements: u64, kernel_calls: u64) {
    println!("SVDAG_WORK\t{name}\t{elements}\t{kernel_calls}");
}

pub fn resource(name: &str, dag: &Dag) {
    if let Some(bytes) = api::retained_bytes(dag) {
        println!("SVDAG_RESOURCE\t{name}\tretained_bytes\t{bytes}");
    }
}
