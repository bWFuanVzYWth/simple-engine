use crate::ZOrder20x3;
use crate::hashdag::{DAG_DEPTH, Error, HashDag, RootId};
use crate::node::{Child, NodeRef};

#[derive(Clone, Copy)]
struct Frame<'a> {
    child: Child,
    node: Option<NodeRef<'a>>,
    origin: [usize; 3],
    level: u8,
    next: u8,
}

impl Frame<'_> {
    const EMPTY: Self = Self {
        child: Child::Leaf(0),
        node: None,
        origin: [0; 3],
        level: 0,
        next: 0,
    };
}

impl HashDag {
    /// 展开对齐区域到 x 最快、其次 y 和 z 的完整密集输出；不分配、不调度。
    ///
    /// `lod` 为边长指数 `0..=20`，每轴起点须按边长对齐，输出恰好 `2^(3lod)` 项。
    /// 输入形状和 root 错误在写输出前返回；内部结构错误可能留下部分结果。
    pub fn decode_region(
        &self,
        root: RootId,
        path: ZOrder20x3,
        lod: u8,
        output: &mut [u32],
    ) -> Result<(), Error> {
        let (base, side) = region(path, lod)?;
        let count = side.checked_pow(3).ok_or(Error::CapacityOverflow)?;
        if output.len() != count {
            return Err(Error::InvalidDenseValues);
        }
        let end = base.map(|value| value + side);
        self.decode_walk(root, path, lod, base, end, |low, high, value| {
            for z in low[2]..high[2] {
                for y in low[1]..high[1] {
                    let start = low[0] - base[0] + side * (y - base[1] + side * (z - base[2]));
                    output[start..start + high[0] - low[0]].fill(value);
                }
            }
        })
    }

    /// 展开对齐区域内的一张材料平面；形状和失败语义同 [`Self::decode_region`]。
    ///
    /// `axis` 为法线轴 `0..3`，`plane` 为区域内该轴的坐标；输出恰好 `2^(2lod)` 项，
    /// 列沿 `(axis + 1) % 3`，行沿 `(axis + 2) % 3`。仅遍历与平面相交的子区域。
    pub fn decode_plane(
        &self,
        root: RootId,
        path: ZOrder20x3,
        lod: u8,
        axis: u8,
        plane: usize,
        output: &mut [u32],
    ) -> Result<(), Error> {
        let (base, side) = region(path, lod)?;
        if axis >= 3 || plane >= side {
            return Err(Error::InvalidInput);
        }
        if output.len() != side.checked_pow(2).ok_or(Error::CapacityOverflow)? {
            return Err(Error::InvalidDenseValues);
        }
        let axis = usize::from(axis);
        let column = (axis + 1) % 3;
        let row = (axis + 2) % 3;
        let mut low = base;
        let mut high = base.map(|value| value + side);
        low[axis] += plane;
        high[axis] = low[axis] + 1;
        self.decode_walk(root, path, lod, low, high, |low, high, value| {
            for y in low[row]..high[row] {
                let start = low[column] - base[column] + side * (y - base[row]);
                output[start..start + high[column] - low[column]].fill(value);
            }
        })
    }

    fn decode_walk(
        &self,
        root: RootId,
        path: ZOrder20x3,
        lod: u8,
        low: [usize; 3],
        high: [usize; 3],
        mut fill: impl FnMut([usize; 3], [usize; 3], u32),
    ) -> Result<(), Error> {
        let level = lod.div_ceil(2);
        let root_child = self.root_child(root)?;
        let child = super::descend::to_lod(&self.pools, root_child, path, level)?;
        let side = 1_usize << (2 * level);
        let (x, y, z) = path.decode();
        let origin = [x as usize, y as usize, z as usize].map(|value| value & !(side - 1));
        let mut frames = [Frame::EMPTY; DAG_DEPTH as usize + 1];
        frames[0] = self.decode_frame(child, level, origin)?;
        let mut depth = 1;
        while depth != 0 {
            let frame = &mut frames[depth - 1];
            if let Child::Leaf(value) = frame.child {
                let side = 1_usize << (2 * frame.level);
                let clipped_low = std::array::from_fn(|axis| low[axis].max(frame.origin[axis]));
                let clipped_high =
                    std::array::from_fn(|axis| high[axis].min(frame.origin[axis] + side));
                fill(clipped_low, clipped_high, value);
                depth -= 1;
                continue;
            }
            if frame.next == 64 {
                depth -= 1;
                continue;
            }
            let slot = frame.next;
            frame.next += 1;
            let level = frame.level - 1;
            let side = 1_usize << (2 * level);
            let offsets = [
                (slot & 1) | ((slot >> 2) & 2),
                ((slot >> 1) & 1) | ((slot >> 3) & 2),
                ((slot >> 2) & 1) | ((slot >> 4) & 2),
            ];
            let origin =
                std::array::from_fn(|axis| frame.origin[axis] + usize::from(offsets[axis]) * side);
            if (0..3).any(|axis| origin[axis] >= high[axis] || origin[axis] + side <= low[axis]) {
                continue;
            }
            let child = frame.node.ok_or(Error::BuildFrameMissing)?.child(slot);
            frames[depth] = self.decode_frame(child, level, origin)?;
            depth += 1;
        }
        Ok(())
    }

    fn decode_frame(
        &self,
        child: Child,
        level: u8,
        origin: [usize; 3],
    ) -> Result<Frame<'_>, Error> {
        let node = match child {
            Child::Leaf(_) => None,
            Child::Ptr(index) => {
                let pool_level = level.checked_sub(1).ok_or(Error::NodeMissing(index))?;
                Some(
                    self.pools[usize::from(pool_level)]
                        .get(index)
                        .ok_or(Error::NodeMissing(index))?,
                )
            }
        };
        Ok(Frame {
            child,
            node,
            origin,
            level,
            next: 0,
        })
    }
}

fn region(path: ZOrder20x3, lod: u8) -> Result<([usize; 3], usize), Error> {
    if lod > HashDag::AXIS_BITS {
        return Err(Error::InvalidLod(lod));
    }
    let side = 1_usize
        .checked_shl(u32::from(lod))
        .ok_or(Error::CapacityOverflow)?;
    let (x, y, z) = path.decode();
    let base = [x as usize, y as usize, z as usize];
    if base.iter().any(|value| value & (side - 1) != 0) {
        return Err(Error::InvalidInput);
    }
    Ok((base, side))
}

#[cfg(test)]
mod tests {
    use crate::{DenseRegion, SetCell};

    use super::*;

    fn patterned() -> Result<HashDag, Error> {
        let values: Vec<_> = (0..64_u64.pow(3))
            .map(|code| {
                let (x, y, z) = ZOrder20x3::from_value(code).unwrap().decode();
                x + 100 * y + 10000 * z
            })
            .collect();
        let mut dag = HashDag::new()?;
        dag.set_dense_regions(
            dag.root(),
            &[DenseRegion {
                path: ZOrder20x3::ZERO,
                lod: 6,
                values: &values,
            }],
        )?;
        Ok(dag)
    }

    #[test]
    fn region_decodes_both_lod_parities_in_x_fast_order() -> Result<(), Error> {
        let dag = patterned()?;
        for lod in 0..=6 {
            let side = 1_usize << lod;
            let base = if lod == 6 { 0 } else { 32 };
            let path = ZOrder20x3::encode(base, base, base).unwrap();
            let mut output = vec![u32::MAX; side.pow(3)];
            dag.decode_region(dag.root(), path, lod, &mut output)?;
            for (index, &value) in output.iter().enumerate() {
                let x = base + (index % side) as u32;
                let y = base + ((index / side) % side) as u32;
                let z = base + (index / side.pow(2)) as u32;
                assert_eq!(value, x + 100 * y + 10000 * z);
            }
        }
        Ok(())
    }

    #[test]
    fn planes_preserve_axis_and_row_column_order() -> Result<(), Error> {
        let dag = patterned()?;
        for axis in 0..3_u8 {
            for plane in [0, 27, 63] {
                let mut output = vec![0; 64 * 64];
                dag.decode_plane(dag.root(), ZOrder20x3::ZERO, 6, axis, plane, &mut output)?;
                for (index, &value) in output.iter().enumerate() {
                    let mut xyz = [0_u32; 3];
                    xyz[usize::from(axis)] = plane as u32;
                    xyz[(usize::from(axis) + 1) % 3] = (index % 64) as u32;
                    xyz[(usize::from(axis) + 2) % 3] = (index / 64) as u32;
                    assert_eq!(value, xyz[0] + 100 * xyz[1] + 10000 * xyz[2]);
                }
            }
        }
        Ok(())
    }

    #[test]
    fn uniform_region_preserves_full_value() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        dag.set_cells(
            dag.root(),
            &[SetCell {
                path: ZOrder20x3::ZERO,
                data: u32::MAX,
            }],
        )?;
        let mut output = [0];
        dag.decode_region(dag.root(), ZOrder20x3::ZERO, 0, &mut output)?;
        assert_eq!(output, [u32::MAX]);
        Ok(())
    }

    #[test]
    fn invalid_shape_preserves_output() -> Result<(), Error> {
        let dag = HashDag::new()?;
        let mut output = [91; 8];
        assert!(matches!(
            dag.decode_region(
                dag.root(),
                ZOrder20x3::encode(1, 0, 0).unwrap(),
                1,
                &mut output
            ),
            Err(Error::InvalidInput)
        ));
        assert_eq!(output, [91; 8]);
        Ok(())
    }
}
