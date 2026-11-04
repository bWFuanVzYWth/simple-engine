//! 无状态的八子摘要归约；缓存身份与发布阶段由父模块维护。

use crate::Direction;

const FULL: u64 = u16::MAX as u64;

/// 应用将一个源值解释为渲染样本；库不认识空气、方块目录或透明材质。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Sample {
    /// 是否计入占据源格；非立方模型的此值不代表精确物理体积。
    pub occupied: bool,
    /// 材料保留优先级，较大者优先；同级采用较小材料编号。
    pub priority: u16,
    /// 输出给面策略的材料编号，零值也合法。
    pub material: u32,
}

/// 一个主方向的滤波近似，不是源网格外表面积。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Face {
    /// 本方向代表材料；覆盖为零时回退为整体代表材料。
    pub material: u32,
    /// `0..=65535` 的投影覆盖近似；深层量化可能丢失极细结构。
    pub coverage: u16,
    priority: u16,
}

/// 固定大小、按八子合并的源区域摘要。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Summary {
    occupied: u128,
    uniform: Option<u32>,
    material: u32,
    priority: u16,
    faces: [Face; 6],
}

impl Summary {
    pub(super) const EMPTY: Self = Self {
        occupied: 0,
        uniform: None,
        material: 0,
        priority: 0,
        faces: [Face {
            material: 0,
            coverage: 0,
            priority: 0,
        }; 6],
    };

    /// 区域内被应用分类为占据的精确源格数。
    pub const fn occupied(self) -> u128 {
        self.occupied
    }

    /// 全区域为同一原始源值时返回该值；不是代表材料编号。
    pub const fn uniform(self) -> Option<u32> {
        self.uniform
    }

    /// 确定性整体代表材料；不是精确材料直方图的众数。
    pub const fn material(self) -> u32 {
        self.material
    }

    /// 指定外法线方向的代表材料和覆盖近似。
    pub const fn face(self, direction: Direction) -> Face {
        self.faces[direction as usize]
    }

    pub(super) fn uniform_value(value: u32, lod: u8, sample: Sample) -> Self {
        let coverage = if sample.occupied { u16::MAX } else { 0 };
        Self {
            occupied: if sample.occupied {
                1_u128 << (3 * lod as u32)
            } else {
                0
            },
            uniform: Some(value),
            material: sample.material,
            priority: sample.priority,
            faces: [Face {
                material: sample.material,
                coverage,
                priority: sample.priority,
            }; 6],
        }
    }

    pub(super) fn combine(children: [Self; 8]) -> Self {
        let mut result = Self::EMPTY;
        result.uniform = children[0].uniform;
        let mut winner = None;
        for child in children {
            result.occupied += child.occupied;
            if child.uniform != result.uniform {
                result.uniform = None;
            }
            if child.occupied != 0
                && winner.is_none_or(|previous: Self| {
                    child.priority > previous.priority
                        || (child.priority == previous.priority
                            && child.material < previous.material)
                })
            {
                winner = Some(child);
            }
        }
        let winner = winner.unwrap_or(children[0]);
        result.material = winner.material;
        result.priority = winner.priority;
        for direction in Direction::ALL {
            let bit = 1 << direction.axis();
            let mut total = 0_u64;
            let mut best: Option<(Face, u64)> = None;
            for slot in 0..8 {
                if slot & bit != 0 {
                    continue;
                }
                let (front, back) = if direction.positive() {
                    (slot | bit, slot)
                } else {
                    (slot, slot | bit)
                };
                let front = children[front].faces[direction as usize];
                let back = children[back].faces[direction as usize];
                let fw = front.coverage as u64 * FULL;
                let bw = (FULL - front.coverage as u64) * back.coverage as u64;
                total += (fw + bw + FULL / 2) / FULL;
                for (face, weight) in [(front, fw), (back, bw)] {
                    if weight != 0
                        && best.is_none_or(|(previous, previous_weight)| {
                            face.priority > previous.priority
                                || (face.priority == previous.priority
                                    && (weight > previous_weight
                                        || (weight == previous_weight
                                            && face.material < previous.material)))
                        })
                    {
                        best = Some((face, weight));
                    }
                }
            }
            let mut face = best.map_or(
                Face {
                    material: result.material,
                    coverage: 0,
                    priority: result.priority,
                },
                |(face, _)| face,
            );
            face.coverage = ((total + 2) / 4) as u16;
            result.faces[direction as usize] = face;
        }
        result
    }
}
