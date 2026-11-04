//! 稀疏层级建网基准；计时前以独立单元面 oracle 校验完整覆盖、标签和无重叠。
//!
//! reused 计入树访问、层级面相交、标签映射、最少矩形和输出替换，排除输入构造。
//! build_mesh_drop 另含生成密集输入、构造分离数组树、scratch/输出创建与全部析构；
//! 该人工适配准备成本不代表 HashDag 导入。没有线程池或 GPU；输入、scratch、输出容量单列。
use std::{collections::HashSet, convert::Infallible, hint::black_box};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use voxel_mesh::{
    Quad, SIDE, VOLUME,
    sparse::{self, Block, Cell, Node, Tree},
};

struct Fixture {
    dense: Vec<u32>,
    values: [Vec<[u32; 64]>; 3],
    masks: [Vec<u64>; 3],
    root: Cell,
}

impl Fixture {
    fn new(kind: &str) -> Self {
        let mut fixture = Self {
            dense: Vec::with_capacity(VOLUME),
            values: std::array::from_fn(|_| Vec::new()),
            masks: std::array::from_fn(|_| Vec::new()),
            root: Cell::Uniform(0),
        };
        for z in 0..SIDE {
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let occupied = match kind {
                        "uniform" => true,
                        "terrain" => y < 16 + (x / 8 + z / 8) % 8,
                        "cavity" => {
                            !(8..56).contains(&x) || !(8..56).contains(&y) || !(8..56).contains(&z)
                        }
                        "perforated" => y % 16 == 0 && (x % 8 >= 2 || z % 8 >= 2),
                        _ => unreachable!(),
                    };
                    fixture.dense.push(if occupied { u32::MAX } else { 0 });
                }
            }
        }
        let mut cells = Vec::with_capacity(VOLUME);
        for code in 0..VOLUME {
            let mut point = [0; 3];
            for bit in 0..6 {
                for (axis, coordinate) in point.iter_mut().enumerate() {
                    *coordinate |= ((code >> (3 * bit + axis)) & 1) << bit;
                }
            }
            cells.push(Cell::Uniform(
                fixture.dense[point[0] + SIDE * (point[1] + SIDE * point[2])],
            ));
        }
        for level in 0..3 {
            let mut next = Vec::with_capacity(cells.len() / 64);
            for children in cells.as_chunks::<64>().0 {
                if matches!(children[0], Cell::Uniform(_))
                    && children.iter().all(|&child| child == children[0])
                {
                    next.push(children[0]);
                } else {
                    let mut mask = 0;
                    let values = std::array::from_fn(|slot| match children[slot] {
                        Cell::Uniform(value) => value,
                        Cell::Node(value) => {
                            mask |= 1 << slot;
                            value
                        }
                    });
                    let index = fixture.values[level].len() as u32;
                    fixture.values[level].push(values);
                    fixture.masks[level].push(mask);
                    next.push(Cell::Node(index));
                }
            }
            cells = next;
        }
        fixture.root = cells[0];
        fixture
    }

    fn block(&self) -> Block<'_, Self> {
        Block::new(self, self.root)
    }

    fn retained_bytes(&self) -> usize {
        self.dense.capacity() * 4
            + self
                .values
                .iter()
                .map(|v| v.capacity() * 256)
                .sum::<usize>()
            + self.masks.iter().map(|v| v.capacity() * 8).sum::<usize>()
    }
}

impl Tree for Fixture {
    type Error = Infallible;

    fn node(&self, index: u32, level: u8) -> Result<Node<'_>, Infallible> {
        Ok(Node::new(
            &self.values[level as usize][index as usize],
            self.masks[level as usize][index as usize],
        ))
    }
}

const CASES: [&str; 4] = ["uniform", "terrain", "cavity", "perforated"];

fn verify(fixture: &Fixture, output: &[Quad]) {
    let mut expected = HashSet::new();
    let offsets = [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ];
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let material = fixture.dense[x + SIDE * (y + SIDE * z)];
                if material == 0 {
                    continue;
                }
                for (direction, offset) in offsets.iter().enumerate() {
                    let adjacent = [
                        x as i32 + offset[0],
                        y as i32 + offset[1],
                        z as i32 + offset[2],
                    ];
                    let blocked = adjacent.iter().all(|&value| (0..64).contains(&value))
                        && fixture.dense[adjacent[0] as usize
                            + SIDE * (adjacent[1] as usize + SIDE * adjacent[2] as usize)]
                            != 0;
                    if !blocked {
                        expected.insert(([x as u32, y as u32, z as u32], direction, material));
                    }
                }
            }
        }
    }
    let mut actual = HashSet::new();
    for quad in output {
        let axis = quad.direction.axis();
        let u = (axis + 1) % 3;
        let v = (axis + 2) % 3;
        let mut origin = quad.origin;
        origin[axis] = origin[axis]
            .checked_sub(u32::from(quad.direction.positive()))
            .expect("面须属于中心块");
        assert!(origin.iter().all(|&value| value < SIDE as u32));
        assert!(quad.width > 0 && quad.width <= SIDE as u32 - origin[u]);
        assert!(quad.height > 0 && quad.height <= SIDE as u32 - origin[v]);
        for row in 0..quad.height {
            for column in 0..quad.width {
                let mut source = origin;
                source[u] += column;
                source[v] += row;
                assert!(actual.insert((source, quad.direction as usize, quad.material)));
            }
        }
    }
    assert_eq!(actual, expected);
}

fn mesh(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("mesh/sparse/reused");
    group.throughput(Throughput::Elements(1));
    for kind in CASES {
        let fixture = Fixture::new(kind);
        let mut sparse_scratch = sparse::Scratch::default();
        let mut sparse_output = Vec::<Quad>::new();
        sparse::mesh(
            fixture.block(),
            [None; 6],
            &mut sparse_scratch,
            &mut sparse_output,
        )
        .unwrap();
        verify(&fixture, &sparse_output);
        eprintln!(
            "{kind}: input_retained_bytes={} scratch={} output={} quads={}",
            fixture.retained_bytes(),
            sparse_scratch.retained_bytes(),
            sparse_output.capacity() * size_of::<Quad>(),
            sparse_output.len()
        );
        group.bench_function(kind, |bench| {
            bench.iter(|| {
                sparse::mesh(
                    black_box(fixture.block()),
                    [None; 6],
                    &mut sparse_scratch,
                    &mut sparse_output,
                )
                .unwrap();
                black_box(&sparse_output);
            });
        });
    }
    group.finish();
    let mut group = criterion.benchmark_group("mesh/sparse/build_mesh_drop");
    group.throughput(Throughput::Elements(1));
    for kind in CASES {
        group.bench_function(kind, |bench| {
            bench.iter(|| {
                let fixture = Fixture::new(black_box(kind));
                let mut scratch = sparse::Scratch::default();
                let mut output = Vec::new();
                sparse::mesh(fixture.block(), [None; 6], &mut scratch, &mut output).unwrap();
                black_box(output);
            });
        });
    }
    group.finish();
}

criterion_group!(benches, mesh);
criterion_main!(benches);
