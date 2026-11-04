//! 64³ 分层体素的表面提取与最少矩形分解。
//!
//! [`sparse`] 直接借用树生成同标签矩形面，[`lod`] 提供层级摘要和混合尺度入口。
//! 材料与合并标签为 `u32`；树适配、可见性策略与调度归调用方。
//! 每个任务独占 scratch 与输出；失败后须重试整块，空输出也是完整替换结果。

#![forbid(unsafe_code)]

pub mod lod;
pub mod sparse;

/// chunk 的单轴体素数。
pub const SIDE: usize = 64;
/// 一个完整 chunk 的密集元素数。
pub const VOLUME: usize = SIDE * SIDE * SIDE;

/// 面的外法线方向；数值顺序也定义邻居数组的索引。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Direction {
    /// 朝 x 轴负方向。
    NegX,
    /// 朝 x 轴正方向。
    PosX,
    /// 朝 y 轴负方向。
    NegY,
    /// 朝 y 轴正方向。
    PosY,
    /// 朝 z 轴负方向。
    NegZ,
    /// 朝 z 轴正方向。
    PosZ,
}

impl Direction {
    /// 与邻居数组一致的 `-X,+X,-Y,+Y,-Z,+Z` 遍历顺序。
    pub const ALL: [Self; 6] = [
        Self::NegX,
        Self::PosX,
        Self::NegY,
        Self::PosY,
        Self::NegZ,
        Self::PosZ,
    ];

    /// 外法线所在坐标轴，x/y/z 对应 0/1/2。
    pub const fn axis(self) -> usize {
        self as usize / 2
    }

    /// 外法线是否指向该轴的正方向。
    pub const fn positive(self) -> bool {
        self as usize & 1 != 0
    }
}

/// 同标签矩形面；宽高沿外法线轴的后续两轴，不表示三角形索引。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Quad {
    /// 面平面的整数原点，可处于 chunk 最大边界 64。
    pub origin: [u32; 3],
    /// 面的外法线。
    pub direction: Direction,
    /// 沿 `(axis + 1) % 3` 的长度。
    pub width: u32,
    /// 沿 `(axis + 2) % 3` 的长度。
    pub height: u32,
    /// 可见性策略返回的合并标签。
    pub material: u32,
}

impl Quad {
    /// 从外侧观察为 CCW；宽、高分别沿 `(axis + 1) % 3`、`(axis + 2) % 3`。
    pub const fn corners(self) -> [[u32; 3]; 4] {
        let u = (self.direction.axis() + 1) % 3;
        let v = (self.direction.axis() + 2) % 3;
        let a = self.origin;
        let mut b = a;
        b[u] += self.width;
        let mut c = b;
        c[v] += self.height;
        let mut d = a;
        d[v] += self.height;
        if self.direction.positive() {
            [a, b, c, d]
        } else {
            [a, d, c, b]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_winding_points_outward() {
        for direction in Direction::ALL {
            let corners = Quad {
                origin: [3, 5, 7],
                direction,
                width: 2,
                height: 4,
                material: 1,
            }
            .corners();
            let a = std::array::from_fn::<_, 3, _>(|i| corners[1][i] as i32 - corners[0][i] as i32);
            let b = std::array::from_fn::<_, 3, _>(|i| corners[2][i] as i32 - corners[0][i] as i32);
            let normal = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let mut expected = [0; 3];
            expected[direction.axis()] = if direction.positive() { 8 } else { -8 };
            assert_eq!(normal, expected);
        }
    }
}
