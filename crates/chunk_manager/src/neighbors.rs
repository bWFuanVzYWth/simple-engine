//! 任意层差的面邻居与 AIR 缺口；固定栈逐节点遍历，不复制覆盖集合。

use crate::{Chunk, Handle, State, StateError, state::Topology};

/// 一个区块的六个有向面。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Face {
    /// x 最小面。
    NegativeX,
    /// x 最大面。
    PositiveX,
    /// y 最小面。
    NegativeY,
    /// y 最大面。
    PositiveY,
    /// z 最小面。
    NegativeZ,
    /// z 最大面。
    PositiveZ,
}

impl Face {
    /// 按 xyz、负正方向排列的全部面。
    pub const ALL: [Self; 6] = [
        Self::NegativeX,
        Self::PositiveX,
        Self::NegativeY,
        Self::PositiveY,
        Self::NegativeZ,
        Self::PositiveZ,
    ];
    /// 法向轴索引。
    pub const fn axis(self) -> usize {
        match self {
            Self::NegativeX | Self::PositiveX => 0,
            Self::NegativeY | Self::PositiveY => 1,
            Self::NegativeZ | Self::PositiveZ => 2,
        }
    }
    /// 法向是否朝坐标正方向。
    pub const fn positive(self) -> bool {
        matches!(self, Self::PositiveX | Self::PositiveY | Self::PositiveZ)
    }
}

/// 与查询面相交的覆盖区块；消费方应取双方 AABB 在面上的交集。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaceNeighbor {
    /// 当前叶或 AIR 前缀的完整空间范围。
    pub chunk: Chunk,
    /// 可见叶的资源句柄；`None` 表示尚未覆盖的 AIR 区域。
    pub handle: Option<Handle>,
}

/// 一次有界面邻居遍历结果。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NeighborStep {
    /// 检查了一个内部节点，尚未产出覆盖项。
    Progress,
    /// 产出一个互不重叠的邻接覆盖项。
    Item(FaceNeighbor),
    /// 遍历结束。
    Done,
    /// 所选覆盖视图变化；应丢弃本次结果并重新建立游标。
    Changed,
}

/// 面邻居游标，最多 31 层的固定深度优先栈；每次推进只检查一个节点。
///
/// 当前视图在发布后失效，待确认视图在准备、撤销或发布后失效。它们都不持有资源。
pub struct FaceNeighbors {
    target: Chunk,
    face: Face,
    seek: Option<Chunk>,
    stack: [Chunk; 94],
    len: usize,
    version: u64,
    prepared: bool,
}

impl<R> State<R> {
    /// 枚举一个面另一侧的当前可见叶与 AIR；不要求相邻 LOD 只差一级。
    ///
    /// 查询区块应属于所选 cut。无效层或相邻坐标溢出返回层级错误。
    pub fn face_neighbors(&self, chunk: Chunk, face: Face) -> Result<FaceNeighbors, StateError> {
        FaceNeighbors::new(self, chunk, face, false)
    }

    /// 按全部 pending patch 共同发布后的虚拟 cut 枚举面邻居。
    ///
    /// 直接读取唯一节点与家族锁，不叠加另一份 patch/cut 缓存。
    pub fn prepared_face_neighbors(
        &self,
        chunk: Chunk,
        face: Face,
    ) -> Result<FaceNeighbors, StateError> {
        FaceNeighbors::new(self, chunk, face, true)
    }
}

impl FaceNeighbors {
    fn new<R>(
        state: &State<R>,
        chunk: Chunk,
        face: Face,
        prepared: bool,
    ) -> Result<Self, StateError> {
        if chunk.lod > state.config().max_lod() {
            return Err(StateError::Hierarchy);
        }
        let mut target = chunk;
        target.coord[face.axis()] = target.coord[face.axis()]
            .checked_add(if face.positive() { 1 } else { -1 })
            .ok_or(StateError::Hierarchy)?;
        let scale = 1_i64 << (state.config().max_lod() - target.lod);
        let root = Chunk::new(
            target.coord.map(|value| value.div_euclid(scale)),
            state.config().max_lod(),
        );
        Ok(Self {
            target,
            face,
            seek: Some(root),
            stack: [Chunk::new([0; 3], 0); 94],
            len: 0,
            version: if prepared {
                state.prepared_version
            } else {
                state.coverage_version
            },
            prepared,
        })
    }

    /// 访问至多一个当前节点；空区域同样作为 AIR 项产出，不扫描整个状态。
    pub fn advance<R>(&mut self, state: &State<R>) -> NeighborStep {
        let version = if self.prepared {
            state.prepared_version
        } else {
            state.coverage_version
        };
        if self.version != version {
            return NeighborStep::Changed;
        }
        let seeking = self.seek.is_some();
        let chunk = if let Some(chunk) = self.seek.take() {
            chunk
        } else {
            if self.len == 0 {
                return NeighborStep::Done;
            }
            self.len -= 1;
            self.stack[self.len]
        };
        let handle = state.find(chunk);
        let topology = handle.and_then(|handle| state.view_topology(handle, self.prepared));
        match topology {
            Some(Topology::Leaf) => NeighborStep::Item(FaceNeighbor { chunk, handle }),
            None | Some(Topology::Detached) => NeighborStep::Item(FaceNeighbor {
                chunk,
                handle: None,
            }),
            Some(Topology::Branch) if seeking && chunk.lod > self.target.lod => {
                let lod = chunk.lod - 1;
                let scale = 1_i64 << (lod - self.target.lod);
                self.seek = Some(Chunk::new(
                    self.target.coord.map(|value| value.div_euclid(scale)),
                    lod,
                ));
                NeighborStep::Progress
            }
            Some(Topology::Branch) => {
                debug_assert!(chunk.lod != 0);
                for octant in 0..8 {
                    if ((octant >> self.face.axis()) & 1) == usize::from(!self.face.positive()) {
                        let child = Chunk::new(
                            std::array::from_fn(|axis| {
                                chunk.coord[axis] * 2 + ((octant >> axis) & 1) as i64
                            }),
                            chunk.lod - 1,
                        );
                        self.stack[self.len] = child;
                        self.len += 1;
                    }
                }
                NeighborStep::Progress
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Patch};

    fn activate(state: &mut State<u32>, chunk: Chunk) -> Handle {
        let handle = state.reserve_root(chunk).unwrap();
        let build = state.request(handle).unwrap();
        state.complete(build.ticket, 1).unwrap();
        let patch = state.prepare_activate(handle).unwrap();
        state.publish(patch.token, 0).unwrap();
        handle
    }

    fn split(state: &mut State<u32>, parent: Handle) -> Patch {
        for octant in 0..8 {
            let child = state.reserve_child(parent, octant).unwrap();
            let build = state.request(child).unwrap();
            state.complete(build.ticket, 1).unwrap();
        }
        state.prepare_split(parent).unwrap()
    }

    fn collect(state: &State<u32>, mut cursor: FaceNeighbors) -> Vec<FaceNeighbor> {
        let mut result = Vec::new();
        loop {
            match cursor.advance(state) {
                NeighborStep::Item(neighbor) => result.push(neighbor),
                NeighborStep::Progress => {}
                NeighborStep::Done => return result,
                NeighborStep::Changed => panic!("只读遍历不应失效"),
            }
        }
    }

    #[test]
    fn face_traversal_handles_a_two_level_difference() {
        let mut state = State::new(Config::new(3, 4, 200).unwrap());
        let left = activate(&mut state, Chunk::new([0; 3], 2));
        let right = Chunk::new([1, 0, 0], 2);
        activate(&mut state, right);
        let patch = split(&mut state, left);
        state.publish(patch.token, 0).unwrap();
        let children = state.node(left).unwrap().children;
        for octant in [1, 3, 5, 7] {
            let patch = split(&mut state, children[octant].unwrap());
            state.publish(patch.token, 0).unwrap();
        }
        let neighbors = collect(
            &state,
            state.face_neighbors(right, Face::NegativeX).unwrap(),
        );
        assert_eq!(neighbors.len(), 16);
        assert!(
            neighbors
                .iter()
                .all(|neighbor| neighbor.chunk.lod == 0 && neighbor.handle.is_some())
        );
        let tiles: std::collections::HashSet<_> = neighbors
            .iter()
            .map(|neighbor| [neighbor.chunk.coord[1], neighbor.chunk.coord[2]])
            .collect();
        assert_eq!(tiles.len(), 16);
    }

    #[test]
    fn missing_root_is_an_air_cover() {
        let mut state = State::new(Config::new(3, 4, 10).unwrap());
        let chunk = Chunk::new([0; 3], 2);
        activate(&mut state, chunk);
        for face in Face::ALL {
            let neighbors = collect(&state, state.face_neighbors(chunk, face).unwrap());
            assert_eq!(neighbors.len(), 1);
            assert!(neighbors[0].handle.is_none());
            assert_eq!(neighbors[0].chunk.lod, 2);
        }
    }

    #[test]
    fn prepared_neighbors_project_the_locked_family() {
        let mut state = State::new(Config::new(3, 4, 40).unwrap());
        let left = activate(&mut state, Chunk::new([0; 3], 2));
        let right = Chunk::new([1, 0, 0], 2);
        activate(&mut state, right);
        let current = state.face_neighbors(right, Face::NegativeX).unwrap();
        let patch = split(&mut state, left);
        assert_eq!(collect(&state, current).len(), 1);
        let mut prepared = state
            .prepared_face_neighbors(right, Face::NegativeX)
            .unwrap();
        assert_eq!(
            collect(
                &state,
                state
                    .prepared_face_neighbors(right, Face::NegativeX)
                    .unwrap()
            )
            .len(),
            4
        );
        state.reject_patch(patch.token).unwrap();
        assert_eq!(prepared.advance(&state), NeighborStep::Changed);
    }
}
