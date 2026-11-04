//! 实测毫米几何与 RGB 材质；来源及积分近似见 crate 文档。
//! 天花板保留灯面开孔，方块底面由地板封闭。

/// 针孔位置，单位为毫米。
pub const CAMERA_ORIGIN: [f32; 3] = [278.0, 273.0, -800.0];
/// 相机前向单位向量。
pub const CAMERA_FORWARD: [f32; 3] = [0.0, 0.0, 1.0];
/// 传感器右向半宽/焦距，含 Cornell 图像 X 轴方向。
pub const CAMERA_RIGHT: [f32; 3] = [-0.025 / (2.0 * 0.035), 0.0, 0.0];
/// 传感器上向半高/焦距。
pub const CAMERA_UP: [f32; 3] = [0.0, 0.025 / (2.0 * 0.035), 0.0];
/// 矩形光源的采样原点，单位为毫米。
pub const LIGHT_CORNER: [f32; 3] = [343.0, 548.8, 227.0];
/// 光源第一条边，和 V 一起确定单面法线方向。
pub const LIGHT_EDGE_U: [f32; 3] = [0.0, 0.0, 105.0];
/// 光源第二条边，单位为毫米。
pub const LIGHT_EDGE_V: [f32; 3] = [-130.0, 0.0, 0.0];
/// 按四边形顺序各拆两个三角形后的 primitive 数量。
pub const TRIANGLE_COUNT: usize = QUADS.len() * 2;
/// shader 材质表长度，索引与三角形载荷一致。
pub const MATERIAL_COUNT: usize = MATERIALS.len();

const WHITE: u32 = 0;
const RED: u32 = 1;
const GREEN: u32 = 2;
/// 既发光又漫反射的灯面材质索引。
pub const LIGHT: u32 = 3;

const MATERIALS: [[[f32; 4]; 2]; 4] = [
    [[0.73, 0.73, 0.73, 0.0], [0.0; 4]],
    [[0.65, 0.05, 0.05, 0.0], [0.0; 4]],
    [[0.12, 0.45, 0.15, 0.0], [0.0; 4]],
    [[0.78, 0.78, 0.78, 0.0], [15.0, 15.0, 15.0, 0.0]],
];

struct Quad {
    points: [[f32; 3]; 4],
    material: u32,
}

const fn quad(material: u32, points: [[f32; 3]; 4]) -> Quad {
    Quad { points, material }
}

const QUADS: [Quad; 19] = [
    quad(
        WHITE,
        [
            [552.8, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 559.2],
            [549.6, 0.0, 559.2],
        ],
    ),
    quad(
        WHITE,
        [
            [556.0, 548.8, 0.0],
            [556.0, 548.8, 559.2],
            [343.0, 548.8, 332.0],
            [343.0, 548.8, 227.0],
        ],
    ),
    quad(
        WHITE,
        [
            [556.0, 548.8, 559.2],
            [0.0, 548.8, 559.2],
            [213.0, 548.8, 332.0],
            [343.0, 548.8, 332.0],
        ],
    ),
    quad(
        WHITE,
        [
            [0.0, 548.8, 559.2],
            [0.0, 548.8, 0.0],
            [213.0, 548.8, 227.0],
            [213.0, 548.8, 332.0],
        ],
    ),
    quad(
        WHITE,
        [
            [0.0, 548.8, 0.0],
            [556.0, 548.8, 0.0],
            [343.0, 548.8, 227.0],
            [213.0, 548.8, 227.0],
        ],
    ),
    quad(
        WHITE,
        [
            [549.6, 0.0, 559.2],
            [0.0, 0.0, 559.2],
            [0.0, 548.8, 559.2],
            [556.0, 548.8, 559.2],
        ],
    ),
    quad(
        GREEN,
        [
            [0.0, 0.0, 559.2],
            [0.0, 0.0, 0.0],
            [0.0, 548.8, 0.0],
            [0.0, 548.8, 559.2],
        ],
    ),
    quad(
        RED,
        [
            [552.8, 0.0, 0.0],
            [549.6, 0.0, 559.2],
            [556.0, 548.8, 559.2],
            [556.0, 548.8, 0.0],
        ],
    ),
    quad(
        LIGHT,
        [
            [343.0, 548.8, 227.0],
            [343.0, 548.8, 332.0],
            [213.0, 548.8, 332.0],
            [213.0, 548.8, 227.0],
        ],
    ),
    quad(
        WHITE,
        [
            [130.0, 165.0, 65.0],
            [82.0, 165.0, 225.0],
            [240.0, 165.0, 272.0],
            [290.0, 165.0, 114.0],
        ],
    ),
    quad(
        WHITE,
        [
            [290.0, 0.0, 114.0],
            [290.0, 165.0, 114.0],
            [240.0, 165.0, 272.0],
            [240.0, 0.0, 272.0],
        ],
    ),
    quad(
        WHITE,
        [
            [130.0, 0.0, 65.0],
            [130.0, 165.0, 65.0],
            [290.0, 165.0, 114.0],
            [290.0, 0.0, 114.0],
        ],
    ),
    quad(
        WHITE,
        [
            [82.0, 0.0, 225.0],
            [82.0, 165.0, 225.0],
            [130.0, 165.0, 65.0],
            [130.0, 0.0, 65.0],
        ],
    ),
    quad(
        WHITE,
        [
            [240.0, 0.0, 272.0],
            [240.0, 165.0, 272.0],
            [82.0, 165.0, 225.0],
            [82.0, 0.0, 225.0],
        ],
    ),
    quad(
        WHITE,
        [
            [423.0, 330.0, 247.0],
            [265.0, 330.0, 296.0],
            [314.0, 330.0, 456.0],
            [472.0, 330.0, 406.0],
        ],
    ),
    quad(
        WHITE,
        [
            [423.0, 0.0, 247.0],
            [423.0, 330.0, 247.0],
            [472.0, 330.0, 406.0],
            [472.0, 0.0, 406.0],
        ],
    ),
    quad(
        WHITE,
        [
            [472.0, 0.0, 406.0],
            [472.0, 330.0, 406.0],
            [314.0, 330.0, 456.0],
            [314.0, 0.0, 456.0],
        ],
    ),
    quad(
        WHITE,
        [
            [314.0, 0.0, 456.0],
            [314.0, 330.0, 456.0],
            [265.0, 330.0, 296.0],
            [265.0, 0.0, 296.0],
        ],
    ),
    quad(
        WHITE,
        [
            [265.0, 0.0, 296.0],
            [265.0, 330.0, 296.0],
            [423.0, 330.0, 247.0],
            [423.0, 0.0, 247.0],
        ],
    ),
];

/// 按场景顺序拆分 CCW 三角形；各三角形首顶点 w 保存材质索引的 f32 值，其余为零。
pub fn triangles() -> impl Iterator<Item = [[f32; 4]; 3]> {
    QUADS.iter().flat_map(|quad| {
        [[0, 1, 2], [0, 2, 3]].map(|indices| {
            indices.map(|i| {
                let [x, y, z] = quad.points[i];
                [x, y, z, if i == 0 { quad.material as f32 } else { 0.0 }]
            })
        })
    })
}

/// 将三角形顶点按 little-endian float4 顺序打包，和 primitive index 一一对应。
pub fn triangles_bytes() -> Vec<u8> {
    bytes(triangles().flatten(), TRIANGLE_COUNT * 3)
}

/// 每材质两个 little-endian float4：反射率与发射辐射，第四分量为零。
pub fn materials_bytes() -> Vec<u8> {
    bytes(MATERIALS.into_iter().flatten(), MATERIAL_COUNT * 2)
}

fn bytes(vectors: impl IntoIterator<Item = [f32; 4]>, count: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(count * 16);
    for vector in vectors {
        for value in vector {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal(points: [[f32; 3]; 4]) -> [f32; 3] {
        let u = std::array::from_fn::<_, 3, _>(|i| points[1][i] - points[0][i]);
        let v = std::array::from_fn::<_, 3, _>(|i| points[2][i] - points[0][i]);
        [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ]
    }

    #[test]
    fn normals_face_the_transport_domain() {
        for (i, quad) in QUADS.iter().enumerate() {
            let (center, sign) = match i {
                0..9 => ([278.0, 273.0, 279.6], 1.0),
                9..14 => ([185.5, 82.5, 169.0], -1.0),
                _ => ([368.5, 165.0, 351.25], -1.0),
            };
            let n = normal(quad.points);
            let dot: f32 = (0..3).map(|j| n[j] * (center[j] - quad.points[0][j])).sum();
            assert!(dot * sign > 0.0, "surface {i}");
        }
    }

    #[test]
    fn ceiling_leaves_the_light_aperture() {
        let area: f32 = QUADS[1..5]
            .iter()
            .map(|quad| {
                let [a, b, c, d] = quad.points;
                let first = normal([a, b, c, d])[1].abs();
                let second = normal([a, c, d, b])[1].abs();
                (first + second) * 0.5
            })
            .sum();
        let light_area = LIGHT_EDGE_U[2] * -LIGHT_EDGE_V[0];
        assert!((area + light_area - 556.0 * 559.2).abs() < 0.1);
    }
}
