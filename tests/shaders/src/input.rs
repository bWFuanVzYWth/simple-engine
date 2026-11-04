//! 属性 schema 与代表性负载生成；浮点均编码为原始 `u32` bits。
//!
//! 随机字段来自引擎的固定 ChaCha20 协议，输入布局版本由失败记录的 schema 标识。
//! 这里的 Rust 几何计算仅产生独立对照输入，所有被测算法仍执行生产 Slang 模块。

use simple_engine::gpu_test::{PropertyRng, unit_f32};

/// `known-v1` 的 16 字 fixture：MIS 两个 pdf、Lambertian albedo、法线与方向。
/// 其余字作 float4 对齐填充；固定值分别检验权重、BRDF 与采样 pdf。
pub fn known() -> Vec<u32> {
    [
        1.0f32,
        3.0,
        0.25,
        0.5,
        0.75,
        0.0,
        0.0,
        1.0,
        0.866_025_4,
        0.0,
        0.5,
        0.0,
        0.0,
        0.0,
        0.0,
        0.0,
    ]
    .map(f32::to_bits)
    .to_vec()
}

/// `connection-bound-v1` 的 40 字 fixture：三顶点、重心坐标与两段距离、互逆变换矩阵。
/// 近远距离用于检验连接终点的 traversal 退让随距离增长。
pub fn endpoint_bound() -> Vec<u32> {
    [
        [1.0f32, -2.0, 0.5, 0.0],
        [3.0, -1.0, 1.5, 0.0],
        [0.0, 2.0, 2.5, 0.0],
        [0.25, 0.5, 1000.0, 100000.0],
        [2.0, 1.0, 0.25, 17.0],
        [0.0, 4.0, -0.5, -11.0],
        [0.0, 0.0, 0.5, 5.0],
        [0.5, -0.125, -0.375, -8.0],
        [0.0, 0.25, 0.25, 1.5],
        [0.0, 0.0, 2.0, -10.0],
    ]
    .into_iter()
    .flatten()
    .map(f32::to_bits)
    .collect()
}

/// `connection-degenerate-v1` 的 20 字 fixture，分别构造重合与退让后反向的连接。
/// 五行 float4 携带两端位置、法线、偏移和 traversal weight。
pub fn degenerate() -> Vec<u32> {
    [
        [0.0f32, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.75],
        [0.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 1.0],
        [0.0, 0.0, 0.0, 0.75],
    ]
    .into_iter()
    .flatten()
    .map(f32::to_bits)
    .collect()
}

/// `sampling-v1` 每样本 16 字：法线与 PCG state、二维采样与两 pdf、albedo、填充。
/// 法线均匀取球面，pdf 为 `2^-20..=2^20` 的离散幂；每个绝对索引只取一个随机 block。
pub fn sampling(count: u32, seed: [u8; 32], stream: u64) -> Vec<u32> {
    let mut rng = PropertyRng::new(seed, stream);
    let mut input = Vec::with_capacity(count as usize * 16);
    for index in 0..count {
        let words = rng.words(u64::from(index));
        let normal = normal(words[0], words[1]);
        for value in normal {
            input.push(value.to_bits());
        }
        input.push(words[15]);
        for value in [
            unit_f32(words[2]),
            unit_f32(words[3]),
            2.0f32.powi((words[4] % 41) as i32 - 20),
            2.0f32.powi((words[5] % 41) as i32 - 20),
            unit_f32(words[6]),
            unit_f32(words[7]),
            unit_f32(words[8]),
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
        ] {
            input.push(value.to_bits());
        }
    }
    input
}

fn normal(first: u32, second: u32) -> [f32; 3] {
    let z = f64::from(unit_f32(first)) * 2.0 - 1.0;
    let phi = f64::from(unit_f32(second)) * std::f64::consts::TAU;
    let radius = (1.0 - z * z).sqrt();
    [
        (radius * phi.cos()) as f32,
        (radius * phi.sin()) as f32,
        z as f32,
    ]
}

fn transform([x, y, z]: [f32; 3]) -> [f64; 3] {
    let [x, y, z] = [x, y, z].map(f64::from);
    [
        2.0 * x + y + 0.25 * z + 17.0,
        4.0 * y - 0.5 * z - 11.0,
        0.5 * z + 5.0,
    ]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// `surface-v1` 每样本 48 字：三顶点、重心坐标、位置与误差容限、法线、两组 3×4 变换。
/// 尺度跨 `2^-10..=2^10`，固定非均匀缩放与剪切；位置及法线对照用 f64 独立计算。
pub fn surfaces(count: u32, seed: [u8; 32], stream: u64) -> Vec<u32> {
    let mut rng = PropertyRng::new(seed, stream);
    let mut input = Vec::with_capacity(count as usize * 48);
    for index in 0..count {
        let words = rng.words(u64::from(index));
        let scale = 2.0f32.powi((words[0] % 21) as i32 - 10);
        let a = std::array::from_fn(|axis| (unit_f32(words[axis + 1]) - 0.5) * 16.0 * scale);
        let rotation = (words[4] % 3) as usize;
        let edge1 = [2.0, 0.25, 0.5];
        let edge2 = [0.25, 3.0, -0.5];
        let b = std::array::from_fn(|axis| a[axis] + scale * edge1[(axis + rotation) % 3]);
        let c = std::array::from_fn(|axis| a[axis] + scale * edge2[(axis + rotation) % 3]);
        let mut u = unit_f32(words[5]);
        let mut v = unit_f32(words[6]);
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        let [wa, wb, wc] = [a, b, c].map(transform);
        let position: [f64; 3] = std::array::from_fn(|axis| {
            (1.0 - f64::from(u) - f64::from(v)) * wa[axis]
                + f64::from(u) * wb[axis]
                + f64::from(v) * wc[axis]
        });
        let n = cross(
            std::array::from_fn(|i| wb[i] - wa[i]),
            std::array::from_fn(|i| wc[i] - wa[i]),
        );
        let length = n.iter().map(|value| value * value).sum::<f64>().sqrt();
        let extent = [wa, wb, wc]
            .into_iter()
            .flatten()
            .map(f64::abs)
            .fold(1.0, f64::max);
        for row in [
            [a[0], a[1], a[2], 0.0],
            [b[0], b[1], b[2], 0.0],
            [c[0], c[1], c[2], 0.0],
            [u, v, 0.0, 0.0],
            [
                position[0] as f32,
                position[1] as f32,
                position[2] as f32,
                (extent * 8.0 * f64::from(f32::EPSILON)) as f32,
            ],
            [
                (n[0] / length) as f32,
                (n[1] / length) as f32,
                (n[2] / length) as f32,
                0.0,
            ],
            [2.0, 1.0, 0.25, 17.0],
            [0.0, 4.0, -0.5, -11.0],
            [0.0, 0.0, 0.5, 5.0],
            [0.5, -0.125, -0.375, -8.0],
            [0.0, 0.25, 0.25, 1.5],
            [0.0, 0.0, 2.0, -10.0],
        ] {
            input.extend(row.map(f32::to_bits));
        }
    }
    input
}

/// 连接构造的定向负载，不用于代表所有真实场景。
#[derive(Clone, Copy, Debug)]
pub enum Distribution {
    /// 所有三角形同向，视点相对位置一致。
    Coherent,
    /// 三角形法线在球面分散，增加方向差异。
    Incoherent,
    /// 同向三角形，切向距离远大于法向距离。
    Grazing,
}

/// 每样本 20 字：三顶点、重心坐标和 source 位置，供 `connection_load_main` 批量构造连接。
/// `scale` 同时缩放几何和距离；调用方应选取有限正值及不溢出的批量大小。
pub fn connections(
    count: u32,
    distribution: Distribution,
    scale: f32,
    seed: [u8; 32],
    stream: u64,
) -> Vec<u32> {
    let mut rng = PropertyRng::new(seed, stream);
    let mut input = Vec::with_capacity(count as usize * 20);
    for index in 0..count {
        let words = rng.words(u64::from(index));
        let u = unit_f32(words[0]) * 0.5;
        let v = unit_f32(words[1]) * 0.5;
        let n = match distribution {
            Distribution::Incoherent => normal(words[2], words[3]),
            _ => [0.0, 0.0, 1.0],
        };
        let axis = if n[2].abs() < 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let tangent = cross(axis, n.map(f64::from));
        let length = tangent.iter().map(|v| v * v).sum::<f64>().sqrt();
        let tangent = tangent.map(|v| (v / length) as f32);
        let bitangent = cross(n.map(f64::from), tangent.map(f64::from)).map(|v| v as f32);
        let a: [f32; 3] = std::array::from_fn(|i| (unit_f32(words[4 + i]) - 0.5) * 100.0 * scale);
        let b: [f32; 3] = std::array::from_fn(|i| a[i] + tangent[i] * scale * 2.0);
        let c: [f32; 3] = std::array::from_fn(|i| a[i] + bitangent[i] * scale * 3.0);
        let reach = match distribution {
            Distribution::Grazing => 1e5,
            _ => 20.0,
        };
        let source: [f32; 3] = std::array::from_fn(|i| {
            a[i] + (b[i] - a[i]) * u
                + (c[i] - a[i]) * v
                + scale * (10.0 * n[i] + reach * tangent[i])
        });
        for row in [
            [a[0], a[1], a[2], 0.0],
            [b[0], b[1], b[2], 0.0],
            [c[0], c[1], c[2], 0.0],
            [u, v, 0.0, 0.0],
            [source[0], source[1], source[2], 0.0],
        ] {
            input.extend(row.map(f32::to_bits));
        }
    }
    input
}
