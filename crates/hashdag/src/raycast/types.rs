use super::math::signed_voxel;

/// DAG 局部体素坐标；命中结果每轴位于 `0..(1 << HashDag::AXIS_BITS)`。
pub type VoxelCoord = (u32, u32, u32);

/// 有符号 DAG 局部坐标，可表达 root 外或命中面后方一格。
pub type SignedVoxelCoord = (i32, i32, i32);

/// 体素进入轴；边界时间相等时按 X、Y、Z 顺序推进。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Axis {
    /// X 轴。
    X = 0,
    /// Y 轴。
    Y = 1,
    /// Z 轴。
    Z = 2,
}

impl Axis {
    pub(super) const fn from_usize(value: usize) -> Self {
        match value {
            0 => Self::X,
            1 => Self::Y,
            _ => Self::Z,
        }
    }
}

/// DAG 局部射线；输入必须有限，方向平方长度须至少为 `1e-24`。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    /// 原点，单位为体素；允许位于 root 外。
    pub origin: [f64; 3],
    /// 方向，可非单位长度；遍历前会归一化，平方长度溢出也视为非法。
    pub dir: [f64; 3],
    /// 最大行进距离，按归一化方向以体素为单位，支持 `0 <= max_t < 2^24`。
    ///
    /// 上界来自 u64 Q40 的整数位宽；不支持用浮点最大值表示无限射程。
    /// 不检查此上界，超出支持域的结果不保证正确，也不保证返回 `InvalidInput`。
    pub max_t: f64,
}

/// 当前均匀叶的处理决定；不解释叶值的应用含义。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HitBehavior {
    /// 将当前叶报告为命中。
    Hit,
    /// 继续遍历；`lod > 0` 的均匀叶整体跳过。
    Pass,
    /// 停止遍历，报告阻挡。
    Block,
}

/// 按 DDA 顺序访问到的均匀叶及其首个体素。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RaySample {
    /// 射线到达此叶的首个体素。
    pub voxel: VoxelCoord,
    /// 叶存储的完整值。
    pub value: u32,
    /// 叶 LOD；`0` 为单体素，`n` 为边长 `1 << n`。
    pub lod: u8,
    /// 此射线已访问的均匀叶数，从 1 开始，不是体素步数。
    pub step_count: u32,
}

/// 命中身份、进入面与归一化距离。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VoxelHit {
    /// 权威命中体素身份，不应通过浮点位置反推。
    pub voxel: VoxelCoord,
    /// 命中叶存储的完整值。
    pub value: u32,
    /// 命中叶 LOD；`0` 为单体素，`n` 为边长 `1 << n`。
    pub lod: u8,
    /// 进入轴；`None` 表示距离零且尚未跨面的初始命中，含 root 入口边界。
    pub face_axis: Option<Axis>,
    /// 到命中叶入口的归一化行进距离，仅用于排序与距离查询。
    pub t: f64,
    /// DDA 顺序中命中面后方的体素；初始命中时等于 voxel，不保证值为零。
    pub prev_empty: SignedVoxelCoord,
}

impl VoxelHit {
    /// 是否在距离零处、跨面前直接命中，包含 root 入口边界。
    #[must_use]
    pub const fn is_initial(&self) -> bool {
        self.face_axis.is_none()
    }

    /// 从整数体素坐标推导进入面法线。
    ///
    /// 初始命中、坐标无法表示为 i32 或前一体素不沿指定轴相邻时返回 `None`。
    #[must_use]
    pub const fn entry_normal(&self) -> Option<SignedVoxelCoord> {
        let Some(axis) = self.face_axis else {
            return None;
        };
        let Some(voxel) = signed_voxel(self.voxel) else {
            return None;
        };
        let delta = match (
            self.prev_empty.0.checked_sub(voxel.x),
            self.prev_empty.1.checked_sub(voxel.y),
            self.prev_empty.2.checked_sub(voxel.z),
        ) {
            (Some(x), Some(y), Some(z)) => (x, y, z),
            _ => return None,
        };
        match axis {
            Axis::X if (delta.0 == -1 || delta.0 == 1) && delta.1 == 0 && delta.2 == 0 => {
                Some(delta)
            }
            Axis::Y if delta.0 == 0 && (delta.1 == -1 || delta.1 == 1) && delta.2 == 0 => {
                Some(delta)
            }
            Axis::Z if delta.0 == 0 && delta.1 == 0 && (delta.2 == -1 || delta.2 == 1) => {
                Some(delta)
            }
            _ => None,
        }
    }
}

/// 射线输入的首个校验失败原因，按原点、方向、距离顺序检查。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidInputReason {
    /// 方向含非有限值，或平方长度溢出。
    DirNonFinite,
    /// 方向平方长度小于 `1e-24`。
    DirZero,
    /// 原点含非有限值。
    OriginNonFinite,
    /// 最大距离非有限或为负。
    ReachInvalid,
}

/// 没有命中或阻挡的终止原因。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissReason {
    /// 不穿过 root 或已经离开 root。
    BoundaryExceeded,
    /// 超过最大距离；恰好等于最大距离的访问仍可命中。
    ReachExceeded,
}

/// 阻挡的来源。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockedReason {
    /// predicate 返回 [`HitBehavior::Block`]。
    PredicateBlock,
}

/// 内核无法正常结束的原因，与 root 生命周期校验错误分开。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AbortReason {
    /// 内部指针缺失或层级不合法。
    DagCorruption,
    /// 均匀叶访问次数达到 8192 的单射线上限。
    SafetyExhausted,
}

/// 单条射线的结果；root 生命周期或批输出长度错误通过外层 `Result` 返回。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RaycastResult {
    /// predicate 决定命中。
    Hit(VoxelHit),
    /// predicate 决定停止。
    Blocked {
        /// 触发阻挡的首个体素。
        by_voxel: VoxelCoord,
        /// 阻挡叶的值。
        value: u32,
        /// 阻挡叶的 LOD。
        lod: u8,
        /// 到阻挡叶入口的归一化距离。
        t: f64,
        /// 停止来源。
        reason: BlockedReason,
    },
    /// 未命中。
    Miss {
        /// 终止边界。
        reason: MissReason,
    },
    /// 此射线未开始遍历；同批后续射线仍会处理。
    InvalidInput {
        /// 首个输入错误。
        reason: InvalidInputReason,
    },
    /// 内核异常终止。
    Aborted {
        /// 内部异常原因。
        reason: AbortReason,
    },
}

impl RaycastResult {
    /// 是否报告命中。
    #[must_use]
    pub const fn is_hit(&self) -> bool {
        matches!(self, Self::Hit(_))
    }

    /// 借用命中信息，其他结果返回 `None`。
    #[must_use]
    pub const fn as_hit(&self) -> Option<&VoxelHit> {
        if let Self::Hit(hit) = self {
            Some(hit)
        } else {
            None
        }
    }

    /// 是否由非法输入或内核异常终止。
    #[must_use]
    pub const fn is_anomalous(&self) -> bool {
        matches!(self, Self::InvalidInput { .. } | Self::Aborted { .. })
    }
}
