use super::*;
use std::{collections::HashSet, convert::Infallible};

use crate::{SIDE, VOLUME};

struct Fixture {
    values: [Vec<[u32; 64]>; 3],
    masks: [Vec<u64>; 3],
    root: Cell,
}

impl Fixture {
    fn new(dense: &[u32]) -> Self {
        let mut fixture = Self {
            values: std::array::from_fn(|_| Vec::new()),
            masks: std::array::from_fn(|_| Vec::new()),
            root: Cell::Uniform(0),
        };
        let mut cells = Vec::with_capacity(VOLUME);
        for code in 0..VOLUME {
            let mut point = [0; 3];
            for bit in 0..6 {
                for (axis, coordinate) in point.iter_mut().enumerate() {
                    *coordinate |= ((code >> (3 * bit + axis)) & 1) << bit;
                }
            }
            cells.push(Cell::Uniform(dense[index(point)]));
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
                    let node = fixture.values[level].len() as u32;
                    fixture.values[level].push(values);
                    fixture.masks[level].push(mask);
                    next.push(Cell::Node(node));
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

const fn index(point: [usize; 3]) -> usize {
    point[0] + SIDE * (point[1] + SIDE * point[2])
}

type UnitFace = ([u32; 3], u8, u32);

fn oracle(
    center: &[u32],
    neighbors: [Option<&[u32]>; 6],
    policy: impl Fn(u32, u32) -> Option<u32>,
) -> HashSet<UnitFace> {
    let mut result = HashSet::new();
    for z in 0..SIDE {
        for y in 0..SIDE {
            for x in 0..SIDE {
                let point = [x, y, z];
                let source = center[index(point)];
                for direction in Direction::ALL {
                    let axis = direction.axis();
                    let mut adjacent = point;
                    let next = point[axis] as i32 + if direction.positive() { 1 } else { -1 };
                    let neighbor = if (0..64).contains(&next) {
                        adjacent[axis] = next as usize;
                        center[index(adjacent)]
                    } else {
                        adjacent[axis] = if direction.positive() { 0 } else { 63 };
                        neighbors[direction as usize].map_or(0, |dense| dense[index(adjacent)])
                    };
                    if let Some(label) = policy(source, neighbor) {
                        result.insert((point.map(|v| v as u32), direction as u8, label));
                    }
                }
            }
        }
    }
    result
}

fn expand(quads: &[Quad]) -> HashSet<UnitFace> {
    let mut result = HashSet::new();
    for quad in quads {
        let axis = quad.direction.axis();
        for row in 0..quad.height {
            for column in 0..quad.width {
                let mut source = quad.origin;
                source[axis] -= u32::from(quad.direction.positive());
                source[(axis + 1) % 3] += column;
                source[(axis + 2) % 3] += row;
                assert!(result.insert((source, quad.direction as u8, quad.material)));
            }
        }
    }
    result
}

fn opaque(source: u32, neighbor: u32) -> Option<u32> {
    (source != 0 && neighbor == 0).then_some(source)
}

#[test]
fn hierarchy_matches_independent_unit_face_oracle() {
    let mut dense = vec![0; VOLUME];
    for z in 0..64 {
        for x in 0..64 {
            let height = 5 + (x / 8 + z / 16) % 4;
            for y in 0..height {
                dense[index([x, y, z])] = if (x + z) % 5 == 0 { u32::MAX } else { 65_536 };
            }
        }
    }
    for z in 16..24 {
        for y in 0..5 {
            for x in 8..16 {
                dense[index([x, y, z])] = 0;
            }
        }
    }
    let tree = Fixture::new(&dense);
    let mut output = Vec::new();
    mesh(
        tree.block(),
        [None; 6],
        &mut Scratch::default(),
        &mut output,
    )
    .unwrap();
    assert_eq!(expand(&output), oracle(&dense, [None; 6], opaque));
}

#[test]
fn nonrectangular_t_face_uses_the_minimum_two_rectangles() {
    let mut dense = vec![0; VOLUME];
    for [u, v] in [[1, 0], [0, 1], [1, 1], [2, 1]] {
        dense[index([10 + u, 10 + v, 10])] = 1;
    }
    let tree = Fixture::new(&dense);
    let mut output = Vec::new();
    mesh(
        tree.block(),
        [None; 6],
        &mut Scratch::default(),
        &mut output,
    )
    .unwrap();
    // T 的四个单元不构成矩形，至少需要两个；上帽和横条达到此下界。
    assert_eq!(
        output
            .iter()
            .filter(|quad| quad.direction == Direction::PosZ)
            .count(),
        2
    );
}

#[test]
fn every_neighbor_axis_intersects_partial_boundary_hierarchies() {
    let mut scratch = Scratch::default();
    for direction in Direction::ALL {
        let axis = direction.axis();
        let mut center = vec![0; VOLUME];
        let mut neighbor = vec![0; VOLUME];
        for u in 8..24 {
            for v in 16..32 {
                let mut point = [0; 3];
                point[axis] = if direction.positive() { 63 } else { 0 };
                point[(axis + 1) % 3] = u;
                point[(axis + 2) % 3] = v;
                center[index(point)] = u32::MAX;
                point[axis] = 63 - point[axis];
                if (u + v) % 3 == 0 {
                    neighbor[index(point)] = 65_536;
                }
            }
        }
        let center_tree = Fixture::new(&center);
        let neighbor_tree = Fixture::new(&neighbor);
        let mut neighbors = [None; 6];
        neighbors[direction as usize] = Some(neighbor_tree.block());
        let mut output = Vec::new();
        mesh(center_tree.block(), neighbors, &mut scratch, &mut output).unwrap();
        let mut dense_neighbors = [None; 6];
        dense_neighbors[direction as usize] = Some(neighbor.as_slice());
        assert_eq!(expand(&output), oracle(&center, dense_neighbors, opaque));
    }
}

#[test]
fn policy_preserves_zero_label_and_full_width_materials() {
    let mut dense = vec![0; VOLUME];
    dense[index([3, 5, 7])] = 65_536;
    dense[index([4, 5, 7])] = u32::MAX;
    let tree = Fixture::new(&dense);
    let policy = |source, adjacent| match (source, adjacent) {
        (65_536, u32::MAX) => Some(0),
        (u32::MAX, 65_536) => Some(u32::MAX),
        _ => None,
    };
    let mut output = Vec::new();
    mesh_by(
        tree.block(),
        [None; 6],
        &mut Scratch::default(),
        &mut output,
        policy,
    )
    .unwrap();
    assert_eq!(expand(&output), oracle(&dense, [None; 6], policy));
}

#[test]
fn one_directed_plane_preserves_all_4096_distinct_u32_labels() {
    let mut dense = vec![0; VOLUME];
    for y in 0..64 {
        for x in 0..64 {
            dense[index([x, y, 0])] = u32::MAX - (x + 64 * y) as u32;
        }
    }
    let tree = Fixture::new(&dense);
    let mut output = Vec::new();
    mesh(
        tree.block(),
        [None; 6],
        &mut Scratch::default(),
        &mut output,
    )
    .unwrap();
    let actual: HashSet<_> = expand(&output)
        .into_iter()
        .filter(|face| face.1 == Direction::PosZ as u8)
        .collect();
    let expected: HashSet<_> = (0..64_u32)
        .flat_map(|y| {
            (0..64_u32).map(move |x| ([x, y, 0], Direction::PosZ as u8, u32::MAX - x - 64 * y))
        })
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn equal_uniform_values_can_emit_internal_interfaces() {
    let tree = Fixture::new(&vec![7; VOLUME]);
    let mut output = Vec::new();
    mesh_by(
        tree.block(),
        [None; 6],
        &mut Scratch::default(),
        &mut output,
        |source, neighbor| (source == 7 && neighbor == 7).then_some(0),
    )
    .unwrap();
    assert_eq!(output.len(), 6 * 63);
    assert!(output.iter().all(|quad| {
        quad.width == 64
            && quad.height == 64
            && quad.material == 0
            && (1..64).contains(&quad.origin[quad.direction.axis()])
    }));
}

struct UniformOnly;

impl Tree for UniformOnly {
    type Error = Infallible;

    fn node(&self, _: u32, _: u8) -> Result<Node<'_>, Infallible> {
        panic!("均匀区域不应读取节点")
    }
}

#[test]
fn uniform_root_uses_six_sparse_squares_without_tree_reads() {
    let mut output = Vec::new();
    mesh(
        Block::new(&UniformOnly, Cell::Uniform(u32::MAX)),
        [None; 6],
        &mut Scratch::default(),
        &mut output,
    )
    .unwrap();
    assert_eq!(output.len(), 6);
    assert!(
        output
            .iter()
            .all(|quad| quad.width == 64 && quad.height == 64 && quad.material == u32::MAX)
    );
}

#[test]
fn empty_rebuild_replaces_previous_output() {
    let mut scratch = Scratch::default();
    let mut output = Vec::new();
    mesh(
        Block::new(&UniformOnly, Cell::Uniform(1)),
        [None; 6],
        &mut scratch,
        &mut output,
    )
    .unwrap();
    mesh(
        Block::new(&UniformOnly, Cell::Uniform(0)),
        [None; 6],
        &mut scratch,
        &mut output,
    )
    .unwrap();
    assert!(output.is_empty());
}

#[test]
fn neighbor_faces_are_not_owned_by_empty_center() {
    let neighbor = Block::new(&UniformOnly, Cell::Uniform(1));
    let mut output = Vec::new();
    mesh(
        Block::new(&UniformOnly, Cell::Uniform(0)),
        [Some(neighbor); 6],
        &mut Scratch::default(),
        &mut output,
    )
    .unwrap();
    assert!(output.is_empty());
}

#[derive(Debug, thiserror::Error)]
#[error("节点已释放")]
struct ReadError;

struct Missing;

impl Tree for Missing {
    type Error = ReadError;

    fn node(&self, _: u32, _: u8) -> Result<Node<'_>, ReadError> {
        Err(ReadError)
    }
}

#[test]
fn read_failure_preserves_its_original_source() {
    let error = mesh(
        Block::new(&Missing, Cell::Node(1)),
        [None; 6],
        &mut Scratch::default(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(std::error::Error::source(&error).unwrap().is::<ReadError>());
}

struct Interrupted<'a> {
    tree: &'a Fixture,
    reads: std::cell::Cell<usize>,
}

impl Tree for Interrupted<'_> {
    type Error = ReadError;

    fn node(&self, index: u32, level: u8) -> Result<Node<'_>, ReadError> {
        let reads = self.reads.get() + 1;
        self.reads.set(reads);
        if reads == 4 {
            Err(ReadError)
        } else {
            Ok(self.tree.node(index, level).unwrap())
        }
    }
}

#[test]
fn scratch_recovers_after_a_mid_traversal_read_failure() {
    let mut dense = vec![0; VOLUME];
    dense[index([3, 5, 7])] = 1;
    dense[index([40, 50, 60])] = u32::MAX;
    let tree = Fixture::new(&dense);
    let interrupted = Interrupted {
        tree: &tree,
        reads: std::cell::Cell::new(0),
    };
    let mut scratch = Scratch::default();
    let mut output = Vec::new();
    let failed = mesh(
        Block::new(&interrupted, tree.root),
        [None; 6],
        &mut scratch,
        &mut output,
    );
    assert!(matches!(failed, Err(Error::Read(_))));
    mesh(tree.block(), [None; 6], &mut scratch, &mut output).unwrap();
    assert_eq!(expand(&output), oracle(&dense, [None; 6], opaque));
}

struct TooDeep;

impl Tree for TooDeep {
    type Error = Infallible;

    fn node(&self, _: u32, _: u8) -> Result<Node<'_>, Infallible> {
        Ok(Node::new(&[0; 64], u64::MAX))
    }
}

#[test]
fn overdeep_graph_returns_depth_error_without_recursion() {
    let error = mesh(
        Block::new(&TooDeep, Cell::Node(0)),
        [None; 6],
        &mut Scratch::default(),
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(error, Error::Depth));
}
