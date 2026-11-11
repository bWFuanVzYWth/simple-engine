//! 各级使用独立的最近网格锚点，区块边长为 `2^lod` 个 LOD0 单位。
//! 半格位置向正方向舍入；负坐标采用向负无穷取整。球与区块闭包相交时保留区块，
//! 因而相切的两侧均可入选；实际所有权仍按区块的半开空间划分。
//!
//! 游标不分配内存。一次推进至多检查一个区块候选或一行区间，空候选和阶段切换也显式返回，
//! 调用者应按推进次数计预算，而非只计算有效输出。

/// 层级、球半径和状态容量的输入错误。
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// 层数不在 `1..=31`。
    #[error("LOD 层数须在 1..=31")]
    LodCount,
    /// 每层半径小于四个本级区块。
    #[error("LOD 半径须至少为四个区块")]
    Radius,
    /// 状态容量为零，或无法表示最多 `2 * capacity - 1` 个索引节点。
    #[error("状态容量超出范围")]
    Capacity,
    /// 目标坐标超出 [`Target::LIMIT`]。
    #[error("目标坐标超出范围")]
    Coordinates,
    /// 半开边界必须在每轴满足最小值小于最大值。
    #[error("区块边界须非空")]
    Bounds,
    /// 动态配置不能改变已分配的层数或容量。
    #[error("动态配置不能改变层数或容量")]
    Storage,
}

/// 以 LOD0 网格为单位的整数半开空间边界。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Bounds {
    minimum: [i64; 3],
    maximum: [i64; 3],
}

impl Bounds {
    /// 校验三轴均非空；允许负坐标，不分配内存。
    pub const fn new(minimum: [i64; 3], maximum: [i64; 3]) -> Result<Self, ConfigError> {
        let mut axis = 0;
        while axis < 3 {
            if minimum[axis] >= maximum[axis] {
                return Err(ConfigError::Bounds);
            }
            axis += 1;
        }
        Ok(Self { minimum, maximum })
    }

    /// 包含的最小网格角。
    pub const fn minimum(self) -> [i64; 3] {
        self.minimum
    }

    /// 不包含的最大网格角。
    pub const fn maximum(self) -> [i64; 3] {
        self.maximum
    }

    /// `self \ other` 的至多六个互不重叠的非空半开盒，按轴顺序返回。
    ///
    /// 仅比较端点，不计算跨度或体积，因此也支持 i64 两端；不分配或遍历内部网格。
    pub fn difference(self, other: Self) -> impl ExactSizeIterator<Item = Self> {
        let mut boxes = [self; 6];
        let mut count = 0;
        let low = std::array::from_fn::<_, 3, _>(|i| self.minimum[i].max(other.minimum[i]));
        let high = std::array::from_fn::<_, 3, _>(|i| self.maximum[i].min(other.maximum[i]));
        if (0..3).any(|i| low[i] >= high[i]) {
            count = 1;
        } else {
            let mut middle = self;
            for axis in 0..3 {
                if middle.minimum[axis] < low[axis] {
                    let mut slab = middle;
                    slab.maximum[axis] = low[axis];
                    boxes[count] = slab;
                    count += 1;
                    middle.minimum[axis] = low[axis];
                }
                if high[axis] < middle.maximum[axis] {
                    let mut slab = middle;
                    slab.minimum[axis] = high[axis];
                    boxes[count] = slab;
                    count += 1;
                    middle.maximum[axis] = high[axis];
                }
            }
        }
        boxes.into_iter().take(count)
    }

    /// 区块与边界是否有正体积交集；相切不算相交。
    pub const fn intersects(self, chunk: Chunk) -> bool {
        if chunk.lod > 30 {
            return false;
        }
        let scale = 1_i128 << chunk.lod;
        let mut axis = 0;
        while axis < 3 {
            let lower = chunk.coord[axis] as i128 * scale;
            if lower >= self.maximum[axis] as i128 || lower + scale <= self.minimum[axis] as i128 {
                return false;
            }
            axis += 1;
        }
        true
    }
}

/// 与物理单位无关的三维层级配置。
///
/// `radius` 以各自层级的区块边长为单位，外层根球额外保留两个区块。
/// `capacity` 只声明状态容量，不参与几何裁剪；构造本身不分配内存。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    lod_count: u8,
    radius: u32,
    capacity: usize,
    bounds: Option<Bounds>,
}

impl Config {
    /// 校验层数、半径与 `1..=usize::MAX / 2` 的容量；不预先保证内存分配成功。
    pub const fn new(lod_count: u8, radius: u32, capacity: usize) -> Result<Self, ConfigError> {
        if lod_count == 0 || lod_count > 31 {
            return Err(ConfigError::LodCount);
        }
        if radius < 4 {
            return Err(ConfigError::Radius);
        }
        if capacity == 0 || capacity > usize::MAX / 2 {
            return Err(ConfigError::Capacity);
        }
        Ok(Self {
            lod_count,
            radius,
            capacity,
            bounds: None,
        })
    }

    /// 包含 LOD0 的层数。
    pub const fn lod_count(self) -> u8 {
        self.lod_count
    }

    /// 最大层级，区块边长为 `2^max_lod` 个 LOD0 单位。
    pub const fn max_lod(self) -> u8 {
        self.lod_count - 1
    }

    /// 各级球半径，以对应层级的区块边长为单位。
    pub const fn radius(self) -> u32 {
        self.radius
    }

    /// 调用方为状态指定的槽位容量。
    pub const fn capacity(self) -> usize {
        self.capacity
    }

    /// 返回替换后的槽位容量；不分配内存，已存在状态使用 [`crate::State::grow_capacity`]。
    pub const fn with_capacity(mut self, capacity: usize) -> Result<Self, ConfigError> {
        if capacity == 0 || capacity > usize::MAX / 2 {
            return Err(ConfigError::Capacity);
        }
        self.capacity = capacity;
        Ok(self)
    }

    /// 返回替换后的有效半径，不改变层数、容量或边界。
    pub const fn with_radius(mut self, radius: u32) -> Result<Self, ConfigError> {
        if radius < 4 {
            return Err(ConfigError::Radius);
        }
        self.radius = radius;
        Ok(self)
    }

    /// 限制根分配及内部细分；完整八子家族仍保留边界外的叶。
    ///
    /// 这里不裁剪网格，宿主应为不相交的边界外孩子构建空资源。
    pub const fn with_bounds(mut self, bounds: Option<Bounds>) -> Self {
        self.bounds = bounds;
        self
    }

    /// 当前可选的有限空间边界。
    pub const fn bounds(self) -> Option<Bounds> {
        self.bounds
    }

    /// 区块是否与配置边界相交；无边界时接受所有有效层。
    pub const fn intersects_bounds(self, chunk: Chunk) -> bool {
        if chunk.lod >= self.lod_count {
            return false;
        }
        match self.bounds {
            Some(bounds) => bounds.intersects(chunk),
            None => true,
        }
    }

    /// 目标在指定层的最近整数锚点。
    ///
    /// # Panics
    ///
    /// `lod` 不在配置范围内时 panic。
    pub const fn anchor(self, target: Target, lod: u8) -> [i64; 3] {
        assert!(lod < self.lod_count, "LOD 超出配置范围");
        anchor(target, lod)
    }

    /// 非最细区块与本级目标球相交时要求细分。
    ///
    /// LOD0 和不存在的层返回 `false`；本接口只决定目标几何，不检查异步资源。
    pub const fn should_split(self, chunk: Chunk, target: Target) -> bool {
        chunk.lod != 0
            && chunk.lod < self.lod_count
            && self.intersects_bounds(chunk)
            && intersects(chunk.coord, anchor(target, chunk.lod), self.radius as u64)
    }

    /// 区块是否属于最高层根球；其他层返回 `false`。
    ///
    /// 根球采用 `radius + 2`，为内部细分保留外围覆盖。
    pub const fn contains_root(self, chunk: Chunk, target: Target) -> bool {
        chunk.lod == self.max_lod()
            && self.intersects_bounds(chunk)
            && intersects(
                chunk.coord,
                anchor(target, chunk.lod),
                self.radius as u64 + 2,
            )
    }

    /// 目标到区块 AABB 的距离平方，以 LOD0 单位计。
    ///
    /// 此处使用原始目标位置，而非各级锚点。无效层或无法用 `u128` 表示的
    /// 极远外部区块返回 `u128::MAX`；游标产生的合法区块不会饱和。
    pub const fn distance2(self, chunk: Chunk, target: Target) -> u128 {
        if chunk.lod >= self.lod_count {
            return u128::MAX;
        }
        let scale = 1_i128 << chunk.lod;
        let mut result = 0_u128;
        let mut axis = 0;
        while axis < 3 {
            let lower = chunk.coord[axis] as i128 * scale;
            let upper = lower + scale;
            let point = target.coord[axis] as i128;
            let delta = if point < lower {
                (lower - point) as u128
            } else if point > upper {
                (point - upper) as u128
            } else {
                0
            };
            result = result.saturating_add(delta.saturating_mul(delta));
            axis += 1;
        }
        result
    }
}

/// 稳定区块身份；坐标以本级区块为单位，与资源版本无关。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Chunk {
    /// 区块最小角的整数网格坐标，负坐标同样有效。
    pub coord: [i64; 3],
    /// 层级；有效范围由 [`Config::lod_count`] 限定。
    pub lod: u8,
}

impl Chunk {
    /// 保存区块身份；几何查询会拒绝超出配置的层级。
    pub const fn new(coord: [i64; 3], lod: u8) -> Self {
        Self { coord, lod }
    }
}

/// 已校验的观察目标，以 LOD0 整数网格坐标表示。
///
/// 调用方负责物理单位换算；格内移动无需提交新目标。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    coord: [i64; 3],
}

impl Target {
    /// 每轴允许的绝对值上界；为最大半径和层级的几何运算保留余量。
    pub const LIMIT: i64 = 1_i64 << 50;

    /// 校验三轴坐标；任一绝对值超出 [`Self::LIMIT`] 时返回输入错误。
    pub const fn new(coord: [i64; 3]) -> Result<Self, ConfigError> {
        let mut axis = 0;
        while axis < 3 {
            if coord[axis] < -Self::LIMIT || coord[axis] > Self::LIMIT {
                return Err(ConfigError::Coordinates);
            }
            axis += 1;
        }
        Ok(Self { coord })
    }

    /// 原始 LOD0 整数坐标。
    pub const fn coords(self) -> [i64; 3] {
        self.coord
    }
}

/// 一次有界游标推进；`Progress` 同样消耗一个规划工作单位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStep<T> {
    /// 本次只排除了候选或切换阶段，尚有后续工作。
    Progress,
    /// 本次产生一个有效结果。
    Item(T),
    /// 全部工作已结束；后续推进仍返回此值。
    Done,
}

/// 根球的无分配候选游标，从靠近本级锚点的区块开始。
///
/// 按 AABB 到锚点的 Chebyshev 距离壳枚举，壳内不承诺欧氏距离顺序；
/// 每次最多检查一个整数区块，总候选数为 `O((radius + 2)^3)`。
/// 有限边界直接裁剪各壳的矩形范围，工作量为裁后包围盒体积加其最长边，
/// 不为边界外的大球逐格或逐壳推进。
/// 游标保存目标快照，调用方应在使用结果时重新校验最新目标。
#[derive(Clone, Debug)]
pub struct RootCursor {
    config: Config,
    target: Target,
    center: [i64; 3],
    minimum: [i64; 3],
    maximum: [i64; 3],
    radius: i64,
    shell: i64,
    face: u8,
    first: u8,
    u: i64,
    v: i64,
    done: bool,
}

impl RootCursor {
    /// 常量工作量建立扫描边界，不预先枚举球内区块。
    pub const fn new(config: Config, target: Target) -> Self {
        let center = anchor(target, config.max_lod());
        let radius = config.radius as i64 + 2;
        let mut minimum = [-radius - 1; 3];
        let mut maximum = [radius; 3];
        let mut shell = 0;
        let mut last_shell = 0;
        let mut done = false;
        let mut axis = 0;
        while axis < 3 {
            if let Some(bounds) = config.bounds {
                let scale = 1_i64 << config.max_lod();
                let low = bounds.minimum[axis].div_euclid(scale);
                let high = (bounds.maximum[axis] - 1).div_euclid(scale);
                // 先在绝对坐标内裁剪，避免外部极远 bounds 的减法溢出。
                let low = if low > center[axis] + minimum[axis] {
                    low
                } else {
                    center[axis] + minimum[axis]
                };
                let high = if high < center[axis] + maximum[axis] {
                    high
                } else {
                    center[axis] + maximum[axis]
                };
                if low > high {
                    done = true;
                } else {
                    minimum[axis] = low - center[axis];
                    maximum[axis] = high - center[axis];
                }
            }
            let nearest = if minimum[axis] > 0 {
                minimum[axis]
            } else if maximum[axis] < -1 {
                -maximum[axis] - 1
            } else {
                0
            };
            if nearest > shell {
                shell = nearest;
            }
            let farthest = if maximum[axis] > -minimum[axis] - 1 {
                maximum[axis]
            } else {
                -minimum[axis] - 1
            };
            if farthest > last_shell {
                last_shell = farthest;
            }
            axis += 1;
        }
        let mut result = Self {
            config,
            target,
            center,
            minimum,
            maximum,
            radius: last_shell,
            shell,
            face: 0,
            first: 0,
            u: 0,
            v: 0,
            done,
        };
        if shell != 0 {
            result.reset_face();
        }
        result
    }

    /// 检查一个候选并推进，空候选返回 [`CursorStep::Progress`]。
    pub fn advance(&mut self) -> CursorStep<Chunk> {
        if self.done {
            return CursorStep::Done;
        }
        let current = self.current();
        self.increment();
        let Some(current) = current else {
            return CursorStep::Progress;
        };
        let chunk = Chunk::new(current, self.config.max_lod());
        if self.config.contains_root(chunk, self.target) {
            CursorStep::Item(chunk)
        } else {
            CursorStep::Progress
        }
    }

    fn current(&self) -> Option<[i64; 3]> {
        let offset = if self.shell == 0 {
            [
                (self.first & 1) as i64 - 1,
                ((self.first >> 1) & 1) as i64 - 1,
                ((self.first >> 2) & 1) as i64 - 1,
            ]
        } else {
            self.face_limits()?;
            let fixed = if self.face & 1 == 0 {
                -self.shell - 1
            } else {
                self.shell
            };
            match self.face / 2 {
                0 => [fixed, self.u, self.v],
                1 => [self.u, fixed, self.v],
                _ => [self.u, self.v, fixed],
            }
        };
        if (0..3).any(|axis| offset[axis] < self.minimum[axis] || offset[axis] > self.maximum[axis])
        {
            return None;
        }
        Some([
            self.center[0] + offset[0],
            self.center[1] + offset[1],
            self.center[2] + offset[2],
        ])
    }

    fn increment(&mut self) {
        if self.shell == 0 {
            self.first += 1;
            if self.first == 8 {
                self.shell = 1;
                if self.shell > self.radius {
                    self.done = true;
                    return;
                }
                self.reset_face();
            }
            return;
        }
        if let Some((minimum_u, maximum_u, _, maximum_v)) = self.face_limits() {
            if self.u < maximum_u {
                self.u += 1;
                return;
            } else if self.v < maximum_v {
                self.u = minimum_u;
                self.v += 1;
                return;
            }
        }
        self.face += 1;
        if self.face == 6 {
            self.face = 0;
            self.shell += 1;
            if self.shell > self.radius {
                self.done = true;
                return;
            }
        }
        self.reset_face();
    }

    const fn reset_face(&mut self) {
        if let Some((minimum_u, _, minimum_v, _)) = self.face_limits() {
            self.u = minimum_u;
            self.v = minimum_v;
        }
    }

    const fn face_limits(&self) -> Option<(i64, i64, i64, i64)> {
        let axis = self.face as usize / 2;
        let fixed = if self.face & 1 == 0 {
            -self.shell - 1
        } else {
            self.shell
        };
        if fixed < self.minimum[axis] || fixed > self.maximum[axis] {
            return None;
        }
        let u = if axis == 0 { 1 } else { 0 };
        let v = if axis == 2 { 1 } else { 2 };
        let low_u = if axis == 0 {
            -self.shell - 1
        } else {
            -self.shell
        };
        let high_u = if axis == 0 {
            self.shell
        } else {
            self.shell - 1
        };
        let low_v = if axis == 2 {
            -self.shell
        } else {
            -self.shell - 1
        };
        let high_v = if axis == 2 {
            self.shell - 1
        } else {
            self.shell
        };
        let low_u = if low_u > self.minimum[u] {
            low_u
        } else {
            self.minimum[u]
        };
        let high_u = if high_u < self.maximum[u] {
            high_u
        } else {
            self.maximum[u]
        };
        let low_v = if low_v > self.minimum[v] {
            low_v
        } else {
            self.minimum[v]
        };
        let high_v = if high_v < self.maximum[v] {
            high_v
        } else {
            self.maximum[v]
        };
        if low_u > high_u || low_v > high_v {
            None
        } else {
            Some((low_u, high_u, low_v, high_v))
        }
    }
}

/// 同层两个目标球之间的集合变化。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RootChange {
    /// 新目标需要、旧目标不需要的区块。
    Enter(Chunk),
    /// 旧目标需要、新目标不需要的区块。
    Exit(Chunk),
}

/// 单层球的扫描线差分，不枚举未变化的区块体积。
///
/// 分别推进旧、新球的 `(y, z)` 行，直接计算本行的两个 `x` 区间之差。
/// 新球阶段跳过旧球已经覆盖的行，不重复产出。
/// 总工作量为 `O(radius² + K)`，其中 `K` 是两端集合的对称差；大跳不扫描沿途。
/// 有限边界同时裁剪扫描行和输出区间，半径再大也不扫描边界外的行。
/// 各级独立锚点未变化时立即结束。
#[derive(Clone, Debug)]
pub struct LevelDiffCursor {
    old_center: [i64; 3],
    new_center: [i64; 3],
    old_rows: Rows,
    new_rows: Rows,
    radius: u64,
    x_bounds: Option<(i64, i64)>,
    lod: u8,
    phase: u8,
    scan_new: bool,
    runs: [Run; 4],
    run_index: usize,
    run_count: usize,
}

impl LevelDiffCursor {
    /// 保存层级与两端快照；`root` 选择 `radius + 2`，否则使用 `radius`。
    ///
    /// 构造不分配内存、不扫描行；`root` 不额外限制层级。
    ///
    /// # Panics
    ///
    /// `lod` 不在配置范围内时 panic。
    pub const fn new(
        config: Config,
        previous: Target,
        current: Target,
        lod: u8,
        root: bool,
    ) -> Self {
        let old_center = config.anchor(previous, lod);
        let new_center = config.anchor(current, lod);
        let radius = config.radius as u64 + if root { 2 } else { 0 };
        let same = old_center[0] == new_center[0]
            && old_center[1] == new_center[1]
            && old_center[2] == new_center[2];
        Self {
            old_center,
            new_center,
            old_rows: Rows::new(config, lod, old_center, radius),
            new_rows: Rows::new(config, lod, new_center, radius),
            radius,
            x_bounds: match config.bounds {
                Some(bounds) => {
                    let scale = 1_i64 << lod;
                    Some((
                        bounds.minimum[0].div_euclid(scale),
                        (bounds.maximum[0] - 1).div_euclid(scale),
                    ))
                }
                None => None,
            },
            lod,
            phase: if same { 2 } else { 0 },
            scan_new: old_center[1] != new_center[1] || old_center[2] != new_center[2],
            runs: [Run::EMPTY; 4],
            run_index: 0,
            run_count: 0,
        }
    }

    /// 推进一行、一个结果或一次阶段切换；所有分支均为有界工作。
    ///
    /// 行检查即使没有差分也返回 `Progress`，不会在内部跳过任意多的空行。
    pub fn advance(&mut self) -> CursorStep<RootChange> {
        if self.run_index < self.run_count {
            let run = &mut self.runs[self.run_index];
            let chunk = Chunk::new([run.start, run.y, run.z], self.lod);
            let change = if run.enter {
                RootChange::Enter(chunk)
            } else {
                RootChange::Exit(chunk)
            };
            if run.start == run.end {
                self.run_index += 1;
            } else {
                run.start += 1;
            }
            return CursorStep::Item(change);
        }
        self.run_index = 0;
        self.run_count = 0;
        match self.phase {
            0 => {
                if let Some([y, z]) = self.old_rows.next() {
                    let old = self.interval(self.old_center, y, z);
                    let new = self.interval(self.new_center, y, z);
                    self.append_difference(old, new, y, z, false);
                    self.append_difference(new, old, y, z, true);
                    CursorStep::Progress
                } else if self.scan_new {
                    self.phase = 1;
                    CursorStep::Progress
                } else {
                    self.phase = 2;
                    CursorStep::Done
                }
            }
            1 => {
                if let Some([y, z]) = self.new_rows.next() {
                    if !self.old_rows.contains([y, z]) {
                        let new = self.interval(self.new_center, y, z);
                        self.append_difference(new, None, y, z, true);
                    }
                    CursorStep::Progress
                } else {
                    self.phase = 2;
                    CursorStep::Done
                }
            }
            _ => CursorStep::Done,
        }
    }

    fn interval(&self, center: [i64; 3], y: i64, z: i64) -> Option<(i64, i64)> {
        let (mut low, mut high) = row_interval(center, self.radius, y, z)?;
        if let Some((minimum, maximum)) = self.x_bounds {
            low = low.max(minimum);
            high = high.min(maximum);
        }
        (low <= high).then_some((low, high))
    }

    fn append_difference(
        &mut self,
        source: Option<(i64, i64)>,
        other: Option<(i64, i64)>,
        y: i64,
        z: i64,
        enter: bool,
    ) {
        let Some((start, end)) = source else {
            return;
        };
        let Some((other_start, other_end)) = other else {
            self.append_run(start, end, y, z, enter);
            return;
        };
        if end < other_start || other_end < start {
            self.append_run(start, end, y, z, enter);
            return;
        }
        if start < other_start {
            self.append_run(start, other_start - 1, y, z, enter);
        }
        if other_end < end {
            self.append_run(other_end + 1, end, y, z, enter);
        }
    }

    fn append_run(&mut self, start: i64, end: i64, y: i64, z: i64, enter: bool) {
        self.runs[self.run_count] = Run {
            start,
            end,
            y,
            z,
            enter,
        };
        self.run_count += 1;
    }
}

/// 最高层根球的惰性扫描线差分。
#[derive(Clone, Debug)]
pub struct RootDiffCursor {
    inner: LevelDiffCursor,
}

impl RootDiffCursor {
    /// 只保存移动两端，不枚举中间路径或完整差分。
    pub const fn new(config: Config, previous: Target, current: Target) -> Self {
        Self {
            inner: LevelDiffCursor::new(config, previous, current, config.max_lod(), true),
        }
    }

    /// 推进一个有界工作单位；输出中不包含重复区块。
    pub fn advance(&mut self) -> CursorStep<RootChange> {
        self.inner.advance()
    }
}

#[derive(Clone, Copy, Debug)]
struct Run {
    start: i64,
    end: i64,
    y: i64,
    z: i64,
    enter: bool,
}

impl Run {
    const EMPTY: Self = Self {
        start: 0,
        end: 0,
        y: 0,
        z: 0,
        enter: false,
    };
}

#[derive(Clone, Debug)]
struct Rows {
    minimum: [i64; 2],
    maximum: [i64; 2],
    current: [i64; 2],
    done: bool,
}

impl Rows {
    const fn new(config: Config, lod: u8, center: [i64; 3], radius: u64) -> Self {
        let radius = radius as i64;
        let mut minimum = [center[1] - radius - 1, center[2] - radius - 1];
        let mut maximum = [center[1] + radius, center[2] + radius];
        let mut done = false;
        if let Some(bounds) = config.bounds {
            let scale = 1_i64 << lod;
            let mut axis = 0;
            while axis < 2 {
                let low = bounds.minimum[axis + 1].div_euclid(scale);
                let high = (bounds.maximum[axis + 1] - 1).div_euclid(scale);
                if low > minimum[axis] {
                    minimum[axis] = low;
                }
                if high < maximum[axis] {
                    maximum[axis] = high;
                }
                if minimum[axis] > maximum[axis] {
                    done = true;
                }
                axis += 1;
            }
        }
        Self {
            minimum,
            maximum,
            current: minimum,
            done,
        }
    }

    fn contains(&self, row: [i64; 2]) -> bool {
        (0..2).all(|axis| row[axis] >= self.minimum[axis] && row[axis] <= self.maximum[axis])
    }
}

impl Iterator for Rows {
    type Item = [i64; 2];

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let row = self.current;
        if self.current[0] < self.maximum[0] {
            self.current[0] += 1;
        } else if self.current[1] < self.maximum[1] {
            self.current[0] = self.minimum[0];
            self.current[1] += 1;
        } else {
            self.done = true;
        }
        Some(row)
    }
}

fn row_interval(center: [i64; 3], radius: u64, y: i64, z: i64) -> Option<(i64, i64)> {
    let dy = axis_distance(y, center[1]);
    let dz = axis_distance(z, center[2]);
    if dy > radius as u128 || dz > radius as u128 {
        return None;
    }
    let limit = radius as u128 * radius as u128;
    let distance = dy * dy + dz * dz;
    if distance > limit {
        return None;
    }
    let extent = (limit - distance).isqrt() as i64;
    Some((center[0] - extent - 1, center[0] + extent))
}

const fn axis_distance(cell: i64, point: i64) -> u128 {
    let lower = cell as i128;
    let upper = lower + 1;
    let point = point as i128;
    if point < lower {
        (lower - point) as u128
    } else if point > upper {
        (point - upper) as u128
    } else {
        0
    }
}

const fn anchor(target: Target, lod: u8) -> [i64; 3] {
    if lod == 0 {
        return target.coord;
    }
    let scale = 1_i64 << lod;
    let half = scale / 2;
    let mut result = [0; 3];
    let mut axis = 0;
    while axis < 3 {
        result[axis] = (target.coord[axis] + half).div_euclid(scale);
        axis += 1;
    }
    result
}

const fn intersects(coord: [i64; 3], center: [i64; 3], radius: u64) -> bool {
    let limit = radius as u128 * radius as u128;
    let mut distance = 0_u128;
    let mut axis = 0;
    while axis < 3 {
        let lower = coord[axis] as i128;
        let upper = lower + 1;
        let point = center[axis] as i128;
        let delta = if point < lower {
            (lower - point) as u128
        } else if point > upper {
            (point - upper) as u128
        } else {
            0
        };
        if delta > radius as u128 {
            return false;
        }
        distance += delta * delta;
        axis += 1;
    }
    distance <= limit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_use_half_open_intersections() {
        let bounds = Bounds::new([-1; 3], [0; 3]).unwrap();
        assert!(bounds.intersects(Chunk::new([-1; 3], 0)));
        assert!(!bounds.intersects(Chunk::new([0; 3], 0)));
        assert_eq!(Bounds::new([0; 3], [0; 3]), Err(ConfigError::Bounds));
    }
    use std::collections::BTreeSet;

    fn target(coord: [i64; 3]) -> Target {
        Target::new(coord).unwrap()
    }

    fn roots(config: Config, target: Target) -> BTreeSet<Chunk> {
        let mut cursor = RootCursor::new(config, target);
        let mut output = BTreeSet::new();
        loop {
            match cursor.advance() {
                CursorStep::Progress => {}
                CursorStep::Item(chunk) => assert!(output.insert(chunk)),
                CursorStep::Done => break,
            }
        }
        output
    }

    #[test]
    fn rejects_invalid_inputs() {
        assert_eq!(Config::new(0, 4, 1), Err(ConfigError::LodCount));
        assert_eq!(Config::new(32, 4, 1), Err(ConfigError::LodCount));
        assert_eq!(Config::new(1, 3, 1), Err(ConfigError::Radius));
        assert_eq!(Config::new(1, 4, 0), Err(ConfigError::Capacity));
        assert_eq!(Config::new(1, 4, usize::MAX), Err(ConfigError::Capacity));
        assert_eq!(Target::new([i64::MIN, 0, 0]), Err(ConfigError::Coordinates));
    }

    #[test]
    fn negative_nearest_anchor_uses_positive_ties() {
        let config = Config::new(5, 4, 1).unwrap();
        assert_eq!(config.anchor(target([-3, -2, -1]), 1), [-1, -1, 0]);
        assert_eq!(config.anchor(target([-3, -2, -1]), 2), [-1, 0, 0]);
    }

    #[test]
    fn unit_move_changes_at_most_one_coarse_anchor() {
        let config = Config::new(12, 4, 1).unwrap();
        for position in -2048..2048 {
            let before = target([position, 0, 0]);
            let after = target([position + 1, 0, 0]);
            let changed = (1..config.lod_count())
                .filter(|&lod| config.anchor(before, lod) != config.anchor(after, lod))
                .count();
            assert!(changed <= 1, "{position}");
        }
    }

    #[test]
    fn sphere_includes_tangent_boxes() {
        let config = Config::new(2, 4, 1).unwrap();
        let target = target([0; 3]);
        assert!(config.should_split(Chunk::new([4, 0, 0], 1), target));
        assert!(config.should_split(Chunk::new([-5, 0, 0], 1), target));
        assert!(!config.should_split(Chunk::new([4, 1, 0], 1), target));
        assert!(!config.should_split(Chunk::new([0; 3], 0), target));
    }

    #[test]
    fn distance_uses_original_target_and_saturates_extreme_boxes() {
        let config = Config::new(31, 4, 1).unwrap();
        assert_eq!(
            config.distance2(Chunk::new([2, -1, 0], 2), target([7, -5, 2])),
            2
        );
        assert_eq!(
            config.distance2(Chunk::new([i64::MAX; 3], 30), target([0; 3])),
            u128::MAX
        );
    }

    #[test]
    fn root_cursor_matches_independent_sphere_oracle() {
        let config = Config::new(1, 4, 1).unwrap();
        let target = target([-2, 1, -3]);
        let mut expected = BTreeSet::new();
        for z in -10..=5 {
            for y in -7..=9 {
                for x in -10..=6 {
                    let coord = [x, y, z];
                    let distance: i64 = coord
                        .into_iter()
                        .zip(target.coords())
                        .map(|(cell, point)| {
                            let delta = (cell - point).max(point - cell - 1).max(0);
                            delta * delta
                        })
                        .sum();
                    if distance <= 36 {
                        expected.insert(Chunk::new(coord, 0));
                    }
                }
            }
        }
        assert_eq!(roots(config, target), expected);
    }

    #[test]
    fn root_difference_matches_endpoint_sets() {
        let config = Config::new(3, 4, 1).unwrap();
        let before = target([-7, 3, -1]);
        for after in [target([1, -5, 2]), target([8000, -6000, 0])] {
            let old = roots(config, before);
            let new = roots(config, after);
            let mut cursor = RootDiffCursor::new(config, before, after);
            let mut enters = BTreeSet::new();
            let mut exits = BTreeSet::new();
            loop {
                match cursor.advance() {
                    CursorStep::Item(RootChange::Enter(chunk)) => assert!(enters.insert(chunk)),
                    CursorStep::Item(RootChange::Exit(chunk)) => assert!(exits.insert(chunk)),
                    CursorStep::Progress => {}
                    CursorStep::Done => break,
                }
            }
            assert_eq!(enters, new.difference(&old).copied().collect());
            assert_eq!(exits, old.difference(&new).copied().collect());
            assert_eq!(cursor.advance(), CursorStep::Done);
        }
    }

    #[test]
    fn level_difference_matches_split_predicate_in_each_direction() {
        let config = Config::new(4, 4, 1).unwrap();
        let before = target([-8, 3, -1]);
        for after in [
            target([-7, 3, -1]),
            target([-9, 3, -1]),
            target([-8, 4, -1]),
            target([-8, 2, -1]),
            target([-8, 3, 0]),
            target([-8, 3, -2]),
            target([19, -23, 31]),
        ] {
            for lod in 1..config.lod_count() {
                let mut cursor = LevelDiffCursor::new(config, before, after, lod, false);
                let mut actual = BTreeSet::new();
                loop {
                    match cursor.advance() {
                        CursorStep::Item(change) => {
                            let (chunk, entered) = match change {
                                RootChange::Enter(chunk) => (chunk, true),
                                RootChange::Exit(chunk) => (chunk, false),
                            };
                            assert_eq!(config.should_split(chunk, after), entered);
                            assert_eq!(config.should_split(chunk, before), !entered);
                            assert!(actual.insert(chunk));
                        }
                        CursorStep::Progress => {}
                        CursorStep::Done => break,
                    }
                }
                let mut expected = BTreeSet::new();
                for center in [config.anchor(before, lod), config.anchor(after, lod)] {
                    for z in center[2] - 5..=center[2] + 4 {
                        for y in center[1] - 5..=center[1] + 4 {
                            for x in center[0] - 5..=center[0] + 4 {
                                let chunk = Chunk::new([x, y, z], lod);
                                if config.should_split(chunk, before)
                                    != config.should_split(chunk, after)
                                {
                                    expected.insert(chunk);
                                }
                            }
                        }
                    }
                }
                assert_eq!(actual, expected);
            }
        }
    }

    #[test]
    fn unchanged_level_finishes_without_scanning_rows() {
        let config = Config::new(5, 4, 1).unwrap();
        let mut cursor =
            LevelDiffCursor::new(config, target([-1, 0, 0]), target([0, 0, 0]), 4, false);
        assert_eq!(cursor.advance(), CursorStep::Done);
    }

    #[test]
    fn moving_root_difference_visits_rows_instead_of_volume() {
        let config = Config::new(1, 16, 1).unwrap();
        let mut cursor = RootDiffCursor::new(config, target([0; 3]), target([1, 0, 0]));
        let mut empty_steps = 0;
        let mut changes = 0;
        loop {
            match cursor.advance() {
                CursorStep::Progress => empty_steps += 1,
                CursorStep::Item(_) => changes += 1,
                CursorStep::Done => break,
            }
        }
        let diameter = 2 * (config.radius() as usize + 3);
        assert!(empty_steps <= 2 * diameter * diameter + 2);
        assert!(changes > 0);
    }

    #[test]
    fn root_cursor_preserves_near_to_far_buckets() {
        let config = Config::new(1, 4, 1).unwrap();
        let mut cursor = RootCursor::new(config, target([0; 3]));
        for _ in 0..8 {
            let CursorStep::Item(chunk) = cursor.advance() else {
                panic!("起始区块应覆盖锚点");
            };
            assert_eq!(config.distance2(chunk, target([0; 3])), 0);
        }
        let mut previous = 0;
        loop {
            match cursor.advance() {
                CursorStep::Item(chunk) => {
                    let bucket = chunk
                        .coord
                        .into_iter()
                        .map(|cell| cell.max(-cell - 1).max(0))
                        .max()
                        .unwrap();
                    assert!(bucket >= previous);
                    previous = bucket;
                }
                CursorStep::Progress => {}
                CursorStep::Done => break,
            }
        }
        assert_eq!(cursor.advance(), CursorStep::Done);
    }

    #[test]
    fn bounded_roots_skip_the_exterior_of_a_huge_sphere() {
        let bounds = Bounds::new([-2, -1, -2], [2, 2, 3]).unwrap();
        let config = Config::new(1, u32::MAX, 1)
            .unwrap()
            .with_bounds(Some(bounds));
        for position in [[0; 3], [50_000_000, 0, 0], [-50_000_000, 0, 0]] {
            let mut cursor = RootCursor::new(config, target(position));
            let mut found = BTreeSet::new();
            let mut steps = 0;
            loop {
                steps += 1;
                assert!(steps < 100, "有限范围不应扫描远处空壳");
                match cursor.advance() {
                    CursorStep::Item(chunk) => assert!(found.insert(chunk)),
                    CursorStep::Progress => {}
                    CursorStep::Done => break,
                }
            }
            assert_eq!(found.len(), 4 * 3 * 5);
            assert!(found.iter().all(|&chunk| bounds.intersects(chunk)));
        }
        let mut outside = RootCursor::new(config, target([1 << 40; 3]));
        assert_eq!(outside.advance(), CursorStep::Done);
    }

    #[test]
    fn bounded_difference_matches_endpoint_sets_without_exterior_rows() {
        let config = Config::new(1, u32::MAX, 1)
            .unwrap()
            .with_bounds(Some(Bounds::new([-2, -1, -2], [2, 2, 3]).unwrap()));
        let before = target([0; 3]);
        let after = target([1 << 40; 3]);
        let expected = roots(config, before);
        let mut cursor = RootDiffCursor::new(config, before, after);
        let mut exits = BTreeSet::new();
        let mut steps = 0;
        loop {
            steps += 1;
            assert!(steps < 100, "差分不应扫描有限边界外的行");
            match cursor.advance() {
                CursorStep::Item(RootChange::Exit(chunk)) => assert!(exits.insert(chunk)),
                CursorStep::Item(RootChange::Enter(_)) => panic!("新球完全在有限边界外"),
                CursorStep::Progress => {}
                CursorStep::Done => break,
            }
        }
        assert_eq!(exits, expected);
    }

    #[test]
    fn bounded_difference_handles_empty_old_x_intervals() {
        let config = Config::new(1, 4, 1)
            .unwrap()
            .with_bounds(Some(Bounds::new([-2; 3], [3; 3]).unwrap()));
        for (old, new) in [
            ([-100, 0, 0], [0; 3]),
            ([0; 3], [100, 0, 0]),
            ([-4, -5, 0], [5, 4, 1]),
        ] {
            let old = target(old);
            let new = target(new);
            let old_roots = roots(config, old);
            let new_roots = roots(config, new);
            let mut cursor = RootDiffCursor::new(config, old, new);
            let mut enters = BTreeSet::new();
            let mut exits = BTreeSet::new();
            loop {
                match cursor.advance() {
                    CursorStep::Item(RootChange::Enter(chunk)) => assert!(enters.insert(chunk)),
                    CursorStep::Item(RootChange::Exit(chunk)) => assert!(exits.insert(chunk)),
                    CursorStep::Progress => {}
                    CursorStep::Done => break,
                }
            }
            assert_eq!(enters, new_roots.difference(&old_roots).copied().collect());
            assert_eq!(exits, old_roots.difference(&new_roots).copied().collect());
        }
    }

    #[test]
    fn extreme_valid_input_keeps_root_bounds_and_distance_exact() {
        let config = Config::new(31, u32::MAX, 1).unwrap();
        let target = target([Target::LIMIT, -Target::LIMIT, 0]);
        let mut cursor = RootCursor::new(config, target);
        assert!(matches!(cursor.advance(), CursorStep::Item(_)));
        let center = config.anchor(target, config.max_lod());
        let radius = config.radius() as i64 + 2;
        let corner = Chunk::new(center.map(|axis| axis - radius - 1), config.max_lod());
        assert_ne!(config.distance2(corner, target), u128::MAX);
    }
}
