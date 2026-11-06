//! quad 灯的空间 median BVH，以及供 CPU/GPU 共用的功率距离采样数据。
//!
//! [`Tree`] 不拥有材质、颜色空间或设备；调用方提供正的 proposal power，通常是面积乘
//! 发光亮度。顶点按 `(0,1,2)`、`(0,2,3)` 两个三角形解释，可有一个零面积三角形。
//! 拓扑只取决于 AABB 中心和原输入索引，不使用法线锥。节点中心由真实三角形面积质心、
//! 灯功率加权后存为 f32；内部 power 为两个子节点的 f32 和。
//!
//! 输出按一次分支同时读取两个子节点的访问模式编码 sibling packet，四对一块共享紧密
//! child 区；median 的后代数量由查询携带而不重复存储，协议见 [`Tree::words`]。
//! 这不依赖轴对齐、共面或平行四边形；显式 child 与原输入叶目录保持直接寻址。
//! 正反向采样必须在同一
//! 接收点重放相同的 24-bit 整数分支划分，不能用固定 power 比替代真实 PMF。每个子树
//! 至少保留其后代叶数的 selector，保证全部正功率灯有支持。距离 proposal 不包含方向、
//! coverage 或可见性；这些因素仍由真实输运计算。查询方须保证距离平方可由 f32 表示。
//!
//! 构建串行执行，不隐藏调度；输入共享只读，排序和任务栈由一次构建独占。返回后释放
//! 临时数据和输入借用，树自身不可变。所有遍历及排序均迭代执行。
//!
//! 内部节点的两个子节点以 sibling packet 相邻存储。

#![forbid(unsafe_code)]

mod layout;
mod partition;

pub use layout::{LEAF, SELECTORS};
use layout::{child_offset, leaf_offset, metric_offset, packet_offset};
use partition::select_median;

/// 一个发光 quad；构建时连续消费全部字段，输出索引保持输入顺序。
#[derive(Clone, Copy, Debug)]
pub struct Quad {
    /// 四角坐标，按 `(0,1,2)`、`(0,2,3)` 形成两个三角形；坐标须有限。
    pub vertices: [[f32; 3]; 4],
    /// 正且有限的相对采样功率；不包含接收点距离或方向。
    pub power: f32,
}

/// 构建失败；不产生部分有效的树，分配失败保留底层来源。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 输出或构建临时内存无法申请。
    #[error("灯 BVH 分配失败")]
    Allocation(#[from] std::collections::TryReserveError),
    /// 超过 24-bit selector 可完整覆盖的灯数或地址范围。
    #[error("灯 BVH 容量超限")]
    Capacity,
    /// 指定灯的功率、坐标、三角形面积或备用距离违反输入契约。
    #[error("quad {0} 的几何或功率无效")]
    Quad(usize),
    /// 子树功率之和超出有限 f32。
    #[error("灯 BVH 总功率超出 f32 范围")]
    Power,
}

/// 不借用输入的 quad 灯 BVH；空树没有 root，多灯树从隐式 root packet 0 开始查询。
#[derive(Debug, Default)]
pub struct Tree {
    words: Vec<u32>,
    lights: u32,
}

#[derive(Clone, Copy, Default)]
struct Node {
    centroid: [f64; 3],
    power: f64,
}

#[derive(Clone, Copy)]
struct Task {
    node: usize,
    begin: usize,
    end: usize,
    path: u32,
    depth: u32,
}

impl Tree {
    /// 消费树并移交同一份GPU编码，不复制节点；灯数和原输入顺序由调用方另行保留。
    pub fn into_words(self) -> Vec<u32> {
        self.words
    }

    /// 构建空间 median 树；空输入合法，不过滤或重新编号任何灯。
    ///
    /// 最多接受 [`SELECTORS`] 个灯。非空 quad 须有正且有限的功率，所有坐标有限，
    /// 两三角形总面积及 `|v1-v0|²+|v3-v0|²` 须为正且可由有限 f32 表示。
    /// 临时所有权限定在本次调用；峰值同时包含输入、输出、每节点 32B 元数据、
    /// 每灯 24B 原质心与 4B 排序索引、至多 25 个任务，不创建线程。
    /// # Errors
    /// 几何/功率无效、总功率溢出、灯数过多或分配失败时返回对应错误。
    pub fn build(quads: &[Quad]) -> Result<Self, Error> {
        if quads.len() > SELECTORS as usize {
            return Err(Error::Capacity);
        }
        let lights = quads.len();
        if lights == 0 {
            return Ok(Self::default());
        }
        let count = lights * 2 - 1;
        let leaves = leaf_offset(lights);
        let mut tree = Self {
            words: filled(leaves + lights * 8, 0)?,
            lights: lights as u32,
        };
        let mut nodes = filled(count, Node::default())?;
        let mut centroids = filled(lights, [0.0; 3])?;
        let mut order = Vec::new();
        order.try_reserve_exact(lights)?;
        for (index, quad) in quads.iter().enumerate() {
            let (bounds, centroid, fallback) = prepare(quad).ok_or(Error::Quad(index))?;
            // 原输入的叶元数据先写在固定目录；拓扑分区不移动它。
            for axis in 0..3 {
                tree.words[leaves + index * 8 + axis] = bounds[0][axis].to_bits();
                tree.words[leaves + index * 8 + 4 + axis] = bounds[1][axis].to_bits();
            }
            tree.words[leaves + index * 8 + 3] = fallback.to_bits();
            centroids[index] = centroid;
            order.push(index as u32);
        }
        let mut tasks = Vec::new();
        tasks.try_reserve_exact(25)?;
        tasks.push(Task {
            node: 0,
            begin: 0,
            end: lights,
            path: 0,
            depth: 0,
        });
        let mut next = 1;
        while let Some(task) = tasks.pop() {
            let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
            for &index in &order[task.begin..task.end] {
                for axis in 0..3 {
                    bounds[0][axis] = bounds[0][axis].min(f32::from_bits(
                        tree.words[leaves + index as usize * 8 + axis],
                    ));
                    bounds[1][axis] = bounds[1][axis].max(f32::from_bits(
                        tree.words[leaves + index as usize * 8 + 4 + axis],
                    ));
                }
            }
            if task.end - task.begin == 1 {
                let index = order[task.begin] as usize;
                nodes[task.node].centroid = centroids[index];
                nodes[task.node].power = f64::from(quads[index].power);
                if task.node != 0 {
                    tree.words[metric_offset(task.node) + 3] = quads[index].power.to_bits();
                    tree.words[child_offset(task.node, lights - 1)] = LEAF | index as u32;
                }
                tree.words[leaves + index * 8 + 7] = task.depth << 27 | task.path;
                continue;
            }
            let axis = (0..3)
                .max_by(|&a, &b| {
                    (f64::from(bounds[1][a]) - f64::from(bounds[0][a]))
                        .total_cmp(&(f64::from(bounds[1][b]) - f64::from(bounds[0][b])))
                })
                .expect("三个坐标轴");
            let middle = (task.end - task.begin) / 2;
            select_median(&mut order[task.begin..task.end], middle, &|a, b| {
                let center = |id: u32| {
                    f64::from(f32::from_bits(tree.words[leaves + id as usize * 8 + axis]))
                        + f64::from(f32::from_bits(
                            tree.words[leaves + id as usize * 8 + 4 + axis],
                        ))
                };
                center(a).total_cmp(&center(b)).then(a.cmp(&b))
            });
            if task.node != 0 {
                tree.words[child_offset(task.node, lights - 1)] = (next >> 1) as u32;
            }
            let split = task.begin + middle;
            let depth = task.depth + 1;
            tasks.push(Task {
                node: next + 1,
                begin: split,
                end: task.end,
                depth,
                path: task.path | 1 << task.depth,
            });
            tasks.push(Task {
                node: next,
                begin: task.begin,
                end: split,
                depth,
                path: task.path,
            });
            next += 2;
        }
        for at in (0..count).rev() {
            let child = if at == 0 {
                if lights == 1 { LEAF } else { 0 }
            } else {
                tree.words[child_offset(at, lights - 1)]
            };
            if child & LEAF == 0 {
                let packet = packet_offset(child as usize);
                let a = child as usize * 2 + 1;
                let b = a + 1;
                let power =
                    f32::from_bits(tree.words[packet + 3]) + f32::from_bits(tree.words[packet + 7]);
                if !power.is_finite() {
                    return Err(Error::Power);
                }
                if at != 0 {
                    tree.words[metric_offset(at) + 3] = power.to_bits();
                }
                let total = nodes[a].power + nodes[b].power;
                nodes[at].power = total;
                for axis in 0..3 {
                    nodes[at].centroid[axis] = nodes[a].centroid[axis]
                        + nodes[b].power / total
                            * (nodes[b].centroid[axis] - nodes[a].centroid[axis]);
                }
            }
            if at != 0 {
                for axis in 0..3 {
                    tree.words[metric_offset(at) + axis] =
                        (nodes[at].centroid[axis] as f32).to_bits();
                }
            }
        }
        Ok(tree)
    }

    /// 原输入灯数，不是节点数。
    pub const fn len(&self) -> u32 {
        self.lights
    }

    /// 是否没有任何灯；空树没有可读取的 packet 或叶记录。
    pub const fn is_empty(&self) -> bool {
        self.lights == 0
    }

    /// 逻辑节点数；非空树为 `2*len()-1`，不等于物理 packet 数。
    pub const fn node_count(&self) -> u32 {
        if self.lights == 0 {
            0
        } else {
            self.lights * 2 - 1
        }
    }

    /// 借用可直接上传的 sibling packet words；f32 字段按 IEEE 754 bits 存储。
    ///
    /// 空树没有 words。设非空树 `L=len()`、`I=L-1`，前 `10I` words 为 I 个 packet，
    /// 每至多四个 packet 一块：先依序存各对左、右 `(center.xyz,power)`，每对 8 words；
    /// 再依序存各对 `(left.child,right.child)`，每对 2 words。完整块 160B，末块仅存
    /// 实际对数。packet `p` 的块号 `b=p/4`、块内号 `r=p%4`、块内对数
    /// `n=min(4,I-4b)`，metrics 从 word `40b+8r` 开始，child 从 `40b+8n+2r`
    /// 开始。metrics 保持 32B、child 保持 8B 的编码偏移对齐，不给单节点填充。
    ///
    /// 内部 child 是 packet 索引；叶为 [`LEAF`] 加原输入索引。多灯树从 packet 0
    /// 开始；携带当前子树叶数 N，左右叶数分别为 `N/2`、`N-N/2`。这一推导依赖空间
    /// median 的固定二分，不能用于其他拓扑。单灯树直接选灯 0，无需读取 packet。
    ///
    /// 之后仅将叶区起点向上对齐到 8 words。每灯按原输入顺序
    /// 存两个 `uint4`：`(lower.xyz,|v1-v0|²+|v3-v0|²)`、`(upper.xyz,path)`。
    /// 非空输出总长为 `roundup(40I,32)+32L` bytes。GPU 查询的 base
    /// 以 `uint4` 为单位，放在 buffer 的偶数 base 可保持叶区 32B 对齐；这里只保证
    /// 编码偏移，不要求 CPU 的 `&[u32]` 指针具有额外对齐。
    ///
    /// 路径低 27 bits 从 root 起逐位表示左右分支，高 5 bits 是深度，最多 24。
    ///
    /// 内部距离用 center 的距离平方；叶先尝试 AABB 距离平方，为零则尝试
    /// center 距离平方，仍为零使用叶记录的备用距离。分支评分是 power/distance²：
    /// power 与距离分别除以两子的最大值后交叉相乘；两距离均零时仅按 power。
    /// score 和非正时按负评分回退；最终概率 clamp 到 `[1e-6,1-1e-6]`。
    /// `left=round_ties_even(count*p)` 后 clamp 到 `[left.leaves,count-right.leaves]`，
    /// 初始 count 为 [`SELECTORS`]，真实叶 PMF 为最终 count/SELECTORS。
    pub const fn words(&self) -> &[u32] {
        self.words.as_slice()
    }
}

fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, Error> {
    let mut values = Vec::new();
    values.try_reserve_exact(len)?;
    values.resize(len, value);
    Ok(values)
}

fn prepare(quad: &Quad) -> Option<([[f32; 3]; 2], [f64; 3], f32)> {
    if !quad.power.is_finite()
        || quad.power <= 0.0
        || quad.vertices.iter().flatten().any(|x| !x.is_finite())
    {
        return None;
    }
    let p = quad.vertices.map(|p| p.map(f64::from));
    let area = |b: usize, c: usize| {
        let u: [f64; 3] = std::array::from_fn(|axis| p[b][axis] - p[0][axis]);
        let v: [f64; 3] = std::array::from_fn(|axis| p[c][axis] - p[0][axis]);
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        cross.into_iter().map(|x| x * x).sum::<f64>().sqrt() * 0.5
    };
    let areas = [area(1, 2), area(2, 3)];
    let total = areas[0] + areas[1];
    let represented_area = total as f32;
    let u: [f32; 3] = std::array::from_fn(|axis| quad.vertices[1][axis] - quad.vertices[0][axis]);
    let v: [f32; 3] = std::array::from_fn(|axis| quad.vertices[3][axis] - quad.vertices[0][axis]);
    let fallback =
        u.into_iter().map(|x| x * x).sum::<f32>() + v.into_iter().map(|x| x * x).sum::<f32>();
    if !represented_area.is_finite()
        || represented_area <= 0.0
        || !fallback.is_finite()
        || fallback <= 0.0
    {
        return None;
    }
    let fraction = areas[0] / total;
    Some((
        [
            std::array::from_fn(|a| {
                quad.vertices
                    .iter()
                    .map(|p| p[a])
                    .reduce(f32::min)
                    .expect("四角")
            }),
            std::array::from_fn(|a| {
                quad.vertices
                    .iter()
                    .map(|p| p[a])
                    .reduce(f32::max)
                    .expect("四角")
            }),
        ],
        std::array::from_fn(|a| {
            (fraction * (p[0][a] + p[1][a] + p[2][a])
                + (1.0 - fraction) * (p[2][a] + p[3][a] + p[0][a]))
                / 3.0
        }),
        fallback,
    ))
}

#[cfg(test)]
mod tests;
