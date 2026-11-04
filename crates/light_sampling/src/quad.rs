use crate::{Error, SELECTORS};

/// 任意四角 quad 的面积分布；两半固定为 `(0,1,2)` 和 `(0,2,3)`。
///
/// 不要求共面、轴对齐、凸或平行四边形；允许恰有一个零面积半片。
/// 正片的选择器数至少为 1，反向 PDF 须使用本值内的真实密度，不能假设等于 `1/area`。
#[derive(Clone, Copy, Debug)]
pub struct QuadArea {
    areas: [f64; 2],
    split: u32,
    densities: [f32; 2],
}

impl QuadArea {
    /// 校验四角并构建分布，不保存或复制四角。
    ///
    /// 坐标须有限；每个正片的 f32 边、叉积及叉积长度平方须有限且非零，
    /// 面积密度须为正有限 f32。全退化输入无采样支持，返回 [`Error::Quad`]。
    /// # Errors
    /// 任一几何或数值条件不满足时返回 [`Error::Quad`]。
    pub fn new(vertices: &[[f32; 3]; 4]) -> Result<Self, Error> {
        if vertices.iter().flatten().any(|v| !v.is_finite()) {
            return Err(Error::Quad);
        }
        let mut areas = [0.0; 2];
        for (half, indices) in [[0, 1, 2], [0, 2, 3]].into_iter().enumerate() {
            let [a, b, c] = indices.map(|i| vertices[i]);
            let e1 = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
            let e2 = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
            let cross = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let norm2 = cross.iter().map(|v| v * v).sum::<f32>();
            let e1 = std::array::from_fn::<_, 3, _>(|i| f64::from(b[i]) - f64::from(a[i]));
            let e2 = std::array::from_fn::<_, 3, _>(|i| f64::from(c[i]) - f64::from(a[i]));
            let exact = [
                e1[1] * e2[2] - e1[2] * e2[1],
                e1[2] * e2[0] - e1[0] * e2[2],
                e1[0] * e2[1] - e1[1] * e2[0],
            ];
            let area = exact.iter().map(|v| v * v).sum::<f64>().sqrt() * 0.5;
            if !norm2.is_finite() || (area > 0.0 && norm2 <= 0.0) {
                return Err(Error::Quad);
            }
            areas[half] = area;
        }
        let area = areas[0] + areas[1];
        if !area.is_finite() || area <= 0.0 {
            return Err(Error::Quad);
        }
        let mut split = (areas[0] / area * f64::from(SELECTORS)).round_ties_even() as u32;
        if areas[0] > 0.0 && areas[1] > 0.0 {
            split = split.clamp(1, SELECTORS - 1);
        }
        let masses = [split, SELECTORS - split];
        let densities = std::array::from_fn(|i| {
            if masses[i] == 0 {
                0.0
            } else {
                (f64::from(masses[i]) / f64::from(SELECTORS) / areas[i]) as f32
            }
        });
        if densities
            .iter()
            .zip(areas)
            .any(|(&density, a)| !density.is_finite() || (a > 0.0 && density <= 0.0))
        {
            return Err(Error::Quad);
        }
        Ok(Self {
            areas,
            split,
            densities,
        })
    }

    /// 两个实际三角形的总面积，供应用计算相对发光功率。
    pub const fn area(self) -> f64 {
        self.areas[0] + self.areas[1]
    }

    /// 两个真实三角形各自的面积；零面积半片保留为0。
    pub const fn areas(self) -> [f64; 2] {
        self.areas
    }

    /// 第一片的实际选择器数；第二片为 `2^24 - split`。
    pub const fn split(self) -> u32 {
        self.split
    }

    /// 每片包含片选择 PMF 的面积密度；退化片为零。
    pub const fn densities(self) -> [f32; 2] {
        self.densities
    }

    /// GPU `uint4` 协议：`[split, asuint(density0), asuint(density1), 0]`。
    /// 最后一项不由通用算法读取，应用可保存自己的属性槽或身份。
    pub const fn words(self) -> [u32; 4] {
        [
            self.split,
            self.densities[0].to_bits(),
            self.densities[1].to_bits(),
            0,
        ]
    }
}
