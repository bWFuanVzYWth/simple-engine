use crate::ZOrder20x3;

use crate::hashdag::{Error, HashDag, RootId};
use crate::node::{Child, NodeIdx};
#[cfg(test)]
use crate::pool::Pool;
#[cfg(verus_only)]
use vstd::prelude::*;

#[path = "read/decode.rs"]
mod decode;
#[path = "read/descend.rs"]
mod descend;

#[cfg(verus_only)]
#[path = "proof/identity.rs"]
mod proof;

/// 子树查询结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubtreeState {
    /// 对齐区域内所有体素具有同一值。
    Uniform {
        /// 该区域的值。
        value: u32,
    },
    /// 区域内至少有两个不同值。
    Mixed,
}

/// 原生节点的内容身份，须与原生层级和槽编号共同作为缓存键。
///
/// 同一存活节点保持不变；槽回收再利用后 generation 不重复。身份不保活节点，
/// 读取仍须持有有效 root。DAG 销毁后可以保留纯派生数据，不能再通过身份读取源。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct NodeIdentity {
    /// 进程内不重复的 DAG 实例身份。
    pub owner: u64,
    /// 本层新节点的单调内容身份；跨层不保证不同。
    pub generation: u64,
}

impl HashDag {
    /// 查询存活节点的稳定内容身份，用于跨编辑的派生缓存。
    ///
    /// 相同内容在仍存活时复用原身份；释放后重新插入允许获得新身份。
    /// 不遍历子树、不分配内存，缓存应另行限制容量。
    ///
    /// # Errors
    ///
    /// 原生层越界或节点已失活时返回错误。
    #[cfg_attr(verus_only, verus_verify)]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires self.valid(),
        ensures self.identity_result(level, index, result),
    ))]
    pub fn node_identity(&self, level: u8, index: NodeIdx) -> Result<NodeIdentity, Error> {
        let generation = self
            .pools
            .get(usize::from(level))
            .ok_or(Error::InvalidLod(level))?
            .generation(index)
            .ok_or(Error::NodeMissing(index))?;
        Ok(NodeIdentity {
            owner: self.root().owner as u64,
            generation,
        })
    }

    /// 返回指定 LOD 池的物理槽位数，包含可复用空槽。
    ///
    /// # Errors
    ///
    /// `lod` 超出 SVDAG 深度时返回错误。
    #[cfg(test)]
    pub(crate) fn pool_slot_count(&self, lod: u8) -> Result<usize, Error> {
        self.pools
            .get(usize::from(lod))
            .map(Pool::slot_count)
            .ok_or(Error::InvalidLod(lod))
    }

    /// 读取 `pools[lod][idx]` 节点的 64 个子槽位；lod 是树层级而非空间边长指数。
    ///
    /// 子节点为 `Ptr(i)` 时，`i` 指向 pools[lod-1]。
    ///
    /// # Errors
    ///
    /// LOD 越界或节点不存在时返回错误。
    pub fn children_of(&self, lod: u8, idx: NodeIdx) -> Result<[Child; 64], Error> {
        let pool = self
            .pools
            .get(usize::from(lod))
            .ok_or(Error::InvalidLod(lod))?;
        let node = pool.get(idx).ok_or(Error::NodeMissing(idx))?;
        Ok(node.children())
    }

    /// 读取指定 root 的单点数据。
    ///
    /// # Errors
    ///
    /// root 失效或 DAG 内部 Ptr 指向缺失节点时返回错误。
    #[inline]
    pub fn value_at(&self, root: RootId, path: ZOrder20x3) -> Result<u32, Error> {
        let mut value = [0];
        self.values_at(root, std::slice::from_ref(&path), &mut value)?;
        Ok(value[0])
    }

    /// 按非降序 Morton 路径读取到等长输出；内核不分配、不调度。
    ///
    /// 复用共享空间前缀的祖先，重复路径广播同一结果；栈空间与 DAG 深度成正比。
    /// 顺序、长度和 root 错误在写输出前返回；内部结构错误可能留下部分输出，
    /// 只有整体成功后才能消费。空批仍验证 root 和输出长度。
    #[inline]
    pub fn values_at(
        &self,
        root: RootId,
        paths: &[ZOrder20x3],
        output: &mut [u32],
    ) -> Result<(), Error> {
        let child = self.root_child(root)?;
        validate_batch(paths, output.len())?;
        if paths.is_empty() {
            return Ok(());
        }
        if let Child::Leaf(value) = child {
            output.fill(value);
            return Ok(());
        }
        if let [path] = paths {
            output[0] = match descend::to_lod(&self.pools, child, *path, 0)? {
                Child::Leaf(value) => Ok(value),
                Child::Ptr(index) => Err(Error::NodeMissing(index)),
            }?;
            return Ok(());
        }
        descend::batch(&self.pools, child, paths, 0, output, |ordered, path| {
            ordered.value(path)
        })
    }

    /// 分类同一空间 LOD 的有序区域，输出形状与失败语义同 [`Self::values_at`]。
    ///
    /// `lod` 为边长指数 `0..=20`；每个 path 向下对齐到所属区域，重复区域只分类一次。
    /// 奇数 LOD 借用节点检查对应八槽，沿用同值叶折叠的 canonical 约束。
    /// 空批仍验证 root、LOD 和输出长度。
    #[inline]
    pub fn subtrees_at(
        &self,
        root: RootId,
        paths: &[ZOrder20x3],
        lod: u8,
        output: &mut [SubtreeState],
    ) -> Result<(), Error> {
        let child = self.root_child(root)?;
        if lod > Self::AXIS_BITS {
            return Err(Error::InvalidLod(lod));
        }
        validate_batch(paths, output.len())?;
        if paths.is_empty() {
            return Ok(());
        }
        if let Child::Leaf(value) = child {
            output.fill(SubtreeState::Uniform { value });
            return Ok(());
        }
        if let [path] = paths {
            let child = descend::to_lod(&self.pools, child, *path, lod.div_ceil(2))?;
            output[0] = descend::state_for_child(&self.pools, child, *path, lod)?;
            return Ok(());
        }
        descend::batch(&self.pools, child, paths, lod, output, |ordered, path| {
            ordered.state(path, lod)
        })
    }

    /// 查询指定 root 中指定子树。
    ///
    /// # Errors
    ///
    /// root 失效、LOD 越界或 DAG 内部 Ptr 指向缺失节点时返回错误。
    #[inline]
    pub fn subtree_at(
        &self,
        root: RootId,
        path: ZOrder20x3,
        lod: u8,
    ) -> Result<SubtreeState, Error> {
        let mut state = [SubtreeState::Mixed];
        self.subtrees_at(root, std::slice::from_ref(&path), lod, &mut state)?;
        Ok(state[0])
    }

    /// 读取指定 root 中指定子树的原始 child。
    /// `lod` 是空间边长的二进制指数；原生子树只接受 0..20 的偶数。
    ///
    /// 这是给上层只读快照与 GPU 打包使用的结构性接口。`Leaf(value)` 的含义仍完全由
    /// 上层解释，SVDAG 不保留空、空气、材质或渲染语义。
    ///
    /// # Errors
    ///
    /// root 失效、LOD 越界或 DAG 内部 Ptr 指向缺失节点时返回错误。
    pub fn subtree_child(&self, root: RootId, path: ZOrder20x3, lod: u8) -> Result<Child, Error> {
        if lod > Self::AXIS_BITS || !lod.is_multiple_of(2) {
            return Err(Error::InvalidLod(lod));
        }
        let root_child = self.root_child(root)?;
        descend::to_lod(&self.pools, root_child, path, lod / 2)
    }

    /// 已初始化的物理节点数组，包括不可达的空闲槽。借用期间禁止编辑；两数组同长。
    /// 数据采用共享读取 ABI，不含 RC、去重表或日志；调用方负责设备复制和版本交接。
    /// # Errors
    /// level 不在 0..10 时返回错误。
    pub fn node_arrays(
        &self,
        level: u8,
    ) -> Result<(&[crate::NodeValues], &[crate::NodeMask]), Error> {
        Ok(self
            .pools
            .get(usize::from(level))
            .ok_or(Error::InvalidLod(level))?
            .arrays())
    }

    /// 读取存活节点中的一个子槽位，level 为 0..10 的 64 叉池层级。
    /// # Errors
    /// 层级、槽位越界或节点不存活时返回错误。
    pub fn child_of(&self, level: u8, index: NodeIdx, slot: u8) -> Result<Child, Error> {
        if slot >= 64 {
            return Err(Error::InvalidInput);
        }
        Ok(self
            .pools
            .get(usize::from(level))
            .ok_or(Error::InvalidLod(level))?
            .get(index)
            .ok_or(Error::NodeMissing(index))?
            .child(slot))
    }
}

#[inline]
fn validate_batch(paths: &[ZOrder20x3], output_len: usize) -> Result<(), Error> {
    if paths.len() != output_len
        || paths
            .windows(2)
            .any(|pair| pair[0].value() > pair[1].value())
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{DenseRegion, SetCell, UniformFill};

    #[test]
    fn physical_arrays_distinguish_full_width_leaves_and_pointers() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let cells = [0, 1, 64, 4096].map(|code| SetCell {
            path: ZOrder20x3::from_value(code).unwrap(),
            data: u32::MAX - code as u32,
        });
        dag.set_cells(dag.root(), &cells)?;
        for cell in cells {
            let mut child = dag.root_child(dag.root())?;
            for level in (0..HashDag::DEPTH).rev() {
                let Child::Ptr(index) = child else { break };
                let slot = ((cell.path.value() >> (u32::from(level) * 6)) & 63) as usize;
                let (values, masks) = dag.node_arrays(level)?;
                let index = index.get() as usize;
                let payload = values[index].words()[slot];
                let pointer = masks[index].words()[slot / 32] & (1 << (slot % 32)) != 0;
                child = if pointer {
                    Child::Ptr(NodeIdx::new(u64::from(payload)).unwrap())
                } else {
                    Child::Leaf(payload)
                };
            }
            assert_eq!(child, Child::Leaf(cell.data));
        }
        Ok(())
    }

    fn path(code: u64) -> ZOrder20x3 {
        ZOrder20x3::from_value(code).unwrap()
    }

    fn dense(values: &[u32], lod: u8) -> Result<HashDag, Error> {
        let mut dag = HashDag::new()?;
        dag.set_dense_regions(
            dag.root(),
            &[DenseRegion {
                path: ZOrder20x3::ZERO,
                lod,
                values,
            }],
        )?;
        Ok(dag)
    }

    #[test]
    fn ordered_points_use_spatial_offsets_inside_shared_nodes() -> Result<(), Error> {
        let values: Vec<_> = (0..4096_u32).map(|code| code % 64).collect();
        let dag = dense(&values, 4)?;
        let codes = [
            0,
            0,
            1,
            63,
            64,
            67,
            127,
            128,
            4095,
            4096,
            1 << 54,
            (1 << 60) - 1,
        ];
        let expected = codes.map(|code| values.get(code as usize).copied().unwrap_or(0));
        let mut output = [u32::MAX; 12];
        dag.values_at(dag.root(), &codes.map(path), &mut output)?;
        assert_eq!(output, expected);
        Ok(())
    }

    #[test]
    fn ordered_prefixes_reduce_node_reads_against_independent_queries() -> Result<(), Error> {
        let values: Vec<_> = (0..4096_u32).map(|code| code % 64).collect();
        let dag = dense(&values, 4)?;
        let root = dag.root_child(dag.root())?;
        let paths: Vec<_> = (0..128).map(path).collect();
        let mut ordered = descend::Ordered::new(&dag.pools, root, 0);
        let output: Vec<_> = paths
            .iter()
            .map(|&path| ordered.seek(path))
            .collect::<Result<_, _>>()?;
        let mut independent_reads = 0;
        let mut independent = Vec::with_capacity(paths.len());
        for &path in &paths {
            let mut query = descend::Ordered::new(&dag.pools, root, 0);
            independent.push(query.seek(path)?);
            independent_reads += query.node_reads;
        }
        assert_eq!(output, independent);
        assert!(ordered.node_reads < independent_reads);
        Ok(())
    }

    #[test]
    fn mixed_state_rejects_missing_target_node() -> Result<(), Error> {
        let dag = HashDag::new()?;
        assert!(matches!(
            descend::state_for_child(&dag.pools, Child::Ptr(NodeIdx::ZERO), path(0), 2),
            Err(Error::NodeMissing(NodeIdx::ZERO))
        ));
        Ok(())
    }

    #[test]
    fn ordered_regions_match_dense_oracle_at_both_lod_parities() -> Result<(), Error> {
        let values: Vec<_> = (0..4096_u32)
            .map(|code| {
                if (code / 512).is_multiple_of(2) {
                    u32::MAX
                } else {
                    code % 13
                }
            })
            .collect();
        let dag = dense(&values, 4)?;
        let paths: Vec<_> = (0..8192_u64)
            .step_by(31)
            .flat_map(|code| [path(code); 2])
            .collect();
        let mut output = vec![SubtreeState::Mixed; paths.len()];
        for lod in 0..=4 {
            dag.subtrees_at(dag.root(), &paths, lod, &mut output)?;
            let width = 1_usize << (3 * lod);
            let expected: Vec<_> = paths
                .iter()
                .map(|path| {
                    let start = path.value() as usize & !(width - 1);
                    let value = values.get(start).copied().unwrap_or(0);
                    if (start..start + width)
                        .all(|code| values.get(code).copied().unwrap_or(0) == value)
                    {
                        SubtreeState::Uniform { value }
                    } else {
                        SubtreeState::Mixed
                    }
                })
                .collect();
            assert_eq!(output, expected);
        }
        Ok(())
    }

    #[test]
    fn uniform_ancestor_covers_all_supported_lods() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        dag.fill_uniform(
            dag.root(),
            &[UniformFill {
                path: ZOrder20x3::ZERO,
                lod: HashDag::AXIS_BITS,
                data: u32::MAX,
            }],
        )?;
        let paths = [path(0), path(1 << 54), path((1 << 60) - 1)];
        for lod in 0..=HashDag::AXIS_BITS {
            let mut output = [SubtreeState::Mixed; 3];
            dag.subtrees_at(dag.root(), &paths, lod, &mut output)?;
            assert_eq!(output, [SubtreeState::Uniform { value: u32::MAX }; 3]);
        }
        Ok(())
    }

    #[test]
    fn invalid_point_order_preserves_output() -> Result<(), Error> {
        let dag = HashDag::new()?;
        let mut output = [91; 2];
        assert!(matches!(
            dag.values_at(dag.root(), &[path(1), path(0)], &mut output),
            Err(Error::InvalidInput)
        ));
        assert_eq!(output, [91; 2]);
        Ok(())
    }

    #[test]
    fn mismatched_region_output_length_preserves_output() -> Result<(), Error> {
        let dag = HashDag::new()?;
        let mut output = [SubtreeState::Mixed; 2];
        assert!(matches!(
            dag.subtrees_at(dag.root(), &[path(0)], 1, &mut output),
            Err(Error::InvalidInput)
        ));
        assert_eq!(output, [SubtreeState::Mixed; 2]);
        Ok(())
    }

    #[test]
    fn empty_batches_still_validate_root_and_lod() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let foreign = HashDag::new()?.root();
        let stale = dag.fork_root(dag.root())?;
        dag.release_root(stale)?;
        assert!(matches!(
            dag.values_at(foreign, &[], &mut []),
            Err(Error::WrongOwner)
        ));
        assert!(matches!(
            dag.subtrees_at(stale, &[], 0, &mut []),
            Err(Error::RootMissing)
        ));
        assert!(matches!(
            dag.subtrees_at(dag.root(), &[], 21, &mut []),
            Err(Error::InvalidLod(21))
        ));
        dag.values_at(dag.root(), &[], &mut [])?;
        dag.subtrees_at(dag.root(), &[], HashDag::AXIS_BITS, &mut [])
    }

    #[test]
    fn reused_node_slot_has_new_content_identity() -> Result<(), Error> {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        let edit = |data| crate::SetCell {
            path: path(0),
            data,
        };
        dag.set_cells(root, &[edit(1)])?;
        let Child::Ptr(before) = dag.subtree_child(root, path(0), 2)? else {
            return Err(Error::InvalidInput);
        };
        let identity = dag.node_identity(0, before)?;
        dag.set_cells(root, &[edit(0)])?;
        assert!(matches!(
            dag.node_identity(0, before),
            Err(Error::NodeMissing(_))
        ));
        dag.set_cells(root, &[edit(2)])?;
        let Child::Ptr(after) = dag.subtree_child(root, path(0), 2)? else {
            return Err(Error::InvalidInput);
        };
        assert_eq!(before, after);
        assert_ne!(identity, dag.node_identity(0, after)?);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn ordered_points_match_independent_dense_values(
            values in prop::collection::vec(any::<u32>(), 512),
            mut codes in prop::collection::vec(0_u64..1024, 0..256),
        ) {
            let dag = dense(&values, 3).map_err(|error| TestCaseError::fail(error.to_string()))?;
            codes.sort_unstable();
            let paths: Vec<_> = codes.iter().copied().map(path).collect();
            let expected: Vec<_> = codes.iter().map(|&code| values.get(code as usize).copied().unwrap_or(0)).collect();
            let mut output = vec![u32::MAX; paths.len()];
            dag.values_at(dag.root(), &paths, &mut output).map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(output, expected);
        }
    }
}
