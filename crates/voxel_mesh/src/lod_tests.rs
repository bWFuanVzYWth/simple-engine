use super::*;
use std::cell::Cell as Counter;

struct RawNode {
    values: [u32; 64],
    pointers: u64,
}
struct TestSource {
    levels: Vec<Vec<RawNode>>,
    versions: Vec<Vec<u64>>,
    reads: Counter<usize>,
}

impl Tree for TestSource {
    type Error = Infallible;
    fn node(&self, index: u32, level: u8) -> Result<Node<'_>, Self::Error> {
        self.reads.set(self.reads.get() + 1);
        let node = &self.levels[level as usize][index as usize];
        Ok(Node::new(&node.values, node.pointers))
    }
}
impl Source for TestSource {
    fn identity(&self, index: u32, level: u8) -> Result<Identity, Self::Error> {
        Ok(Identity {
            owner: 3,
            generation: self.versions[level as usize][index as usize],
        })
    }
}
fn sample(value: u32) -> Sample {
    Sample {
        occupied: value != 0,
        priority: 0,
        material: value,
    }
}
fn stripes(level: u8, empty: bool) -> TestSource {
    let mut levels = Vec::new();
    for current in 0..=level {
        let values = if current == 0 {
            std::array::from_fn(|slot| {
                let x = (slot & 1) | ((slot >> 2) & 2);
                if x < 2 {
                    11
                } else if empty {
                    0
                } else {
                    22
                }
            })
        } else {
            [0; 64]
        };
        levels.push(vec![RawNode {
            values,
            pointers: if current == 0 { 0 } else { u64::MAX },
        }]);
    }
    TestSource {
        versions: vec![vec![1]; levels.len()],
        levels,
        reads: Counter::new(0),
    }
}
fn region(root_lod: u8, lod: u8, minimum: [u64; 3]) -> Region {
    Region {
        root: Cell::Node(0),
        root_lod,
        minimum,
        lod,
    }
}

#[test]
fn occupancy_counts_volume_and_odd_octants_exactly() {
    let source = stripes(2, true);
    let mut pyramid = Pyramid::new();
    let full = pyramid
        .summarize(&source, region(6, 6, [0; 3]), sample)
        .unwrap();
    let half = pyramid
        .summarize(&source, region(6, 5, [32, 0, 0]), sample)
        .unwrap();
    assert_eq!(full.occupied(), 64_u128.pow(3) / 2);
    assert_eq!(half.occupied(), 32_u128.pow(3) / 2);
    assert_eq!(full.uniform(), None);
}

#[test]
fn representative_is_priority_selection_instead_of_majority() {
    let source = TestSource {
        levels: vec![vec![RawNode {
            values: std::array::from_fn(|slot| if slot == 0 { 9 } else { 1 }),
            pointers: 0,
        }]],
        versions: vec![vec![1]],
        reads: Counter::new(0),
    };
    let mut pyramid = Pyramid::new();
    let summary = pyramid
        .summarize(&source, region(2, 2, [0; 3]), |value| Sample {
            priority: if value == 9 { 7 } else { 0 },
            ..sample(value)
        })
        .unwrap();
    assert_eq!(summary.material(), 9);
    assert_eq!(summary.occupied(), 64);
}

#[test]
fn uniform_regions_skip_source_nodes_and_preserve_raw_value() {
    let source = stripes(0, false);
    let mut pyramid = Pyramid::new();
    let request = Region {
        root: Cell::Uniform(73),
        root_lod: 42,
        minimum: [0; 3],
        lod: 42,
    };
    let summary = pyramid
        .summarize(&source, request, |value| Sample {
            material: value + 1,
            ..sample(value)
        })
        .unwrap();
    assert_eq!(summary.occupied(), 1_u128 << 126);
    assert_eq!(summary.uniform(), Some(73));
    assert_eq!(summary.material(), 74);
    assert_eq!(source.reads.get(), 0);
}

#[test]
fn odd_region_build_uses_eight_fine_cells_per_coarse_cell() {
    let source = stripes(3, true);
    let mut pyramid = Pyramid::new();
    let mut tree = LodTree::new();
    pyramid
        .build(&source, region(8, 7, [128, 0, 0]), sample, &mut tree)
        .unwrap();
    assert_eq!(tree.lod(), 1);
    for x in 0..64 {
        assert_eq!(
            tree.sample(&pyramid, [x, 17, 39]).unwrap().occupied(),
            if x % 2 == 0 { 8 } else { 0 }
        );
    }
}

#[test]
fn directional_materials_reach_actual_mesh_labels() {
    let source = stripes(3, false);
    let mut pyramid = Pyramid::new();
    let mut tree = LodTree::new();
    pyramid
        .build(&source, region(8, 8, [0; 3]), sample, &mut tree)
        .unwrap();
    let mut output = Vec::new();
    pyramid
        .mesh_by(
            &tree,
            [None; 6],
            [true; 6],
            &mut sparse::Scratch::default(),
            &mut output,
            |a, b, d| (a.occupied() != 0 && b.occupied() == 0).then_some(a.face(d).material),
        )
        .unwrap();
    assert_eq!(output.len(), 6);
    assert_eq!(
        output
            .iter()
            .find(|quad| quad.direction == Direction::NegX)
            .unwrap()
            .material,
        11
    );
    assert_eq!(
        output
            .iter()
            .find(|quad| quad.direction == Direction::PosX)
            .unwrap()
            .material,
        22
    );
}

#[test]
fn disabled_boundaries_leave_uniform_interior_empty() {
    let source = stripes(0, false);
    let mut pyramid = Pyramid::new();
    let mut tree = LodTree::new();
    pyramid
        .build(
            &source,
            Region {
                root: Cell::Uniform(7),
                root_lod: 6,
                minimum: [0; 3],
                lod: 6,
            },
            sample,
            &mut tree,
        )
        .unwrap();
    let mut output = Vec::new();
    pyramid
        .mesh_by(
            &tree,
            [None; 6],
            [false; 6],
            &mut sparse::Scratch::default(),
            &mut output,
            |a, b, d| (a.occupied() != 0 && b.occupied() == 0).then_some(a.face(d).material),
        )
        .unwrap();
    assert!(output.is_empty());
}

#[test]
fn new_batch_preserves_source_cache_and_invalidates_old_tree() {
    let source = stripes(2, false);
    let mut pyramid = Pyramid::new();
    let request = region(6, 6, [0; 3]);
    let summary = pyramid.summarize(&source, request, sample).unwrap();
    let mut tree = LodTree::new();
    pyramid.build(&source, request, sample, &mut tree).unwrap();
    pyramid.begin_batch();
    source.reads.set(0);
    assert_eq!(
        pyramid.summarize(&source, request, sample).unwrap(),
        summary
    );
    assert_eq!(source.reads.get(), 0);
    assert_eq!(tree.sample(&pyramid, [0; 3]), Err(AccessError::Stale));
}

#[test]
fn one_slot_cache_collisions_do_not_invalidate_output() {
    let source = stripes(3, false);
    let mut pyramid = Pyramid::with_capacity(1);
    let mut tree = LodTree::new();
    pyramid
        .build(&source, region(8, 8, [0; 3]), sample, &mut tree)
        .unwrap();
    let before = *tree.sample(&pyramid, [3, 4, 5]).unwrap();
    pyramid
        .summarize(&source, region(8, 8, [0; 3]), sample)
        .unwrap();
    assert_eq!(*tree.sample(&pyramid, [3, 4, 5]).unwrap(), before);
    assert_eq!(pyramid.cached_nodes(), 1);
}

#[test]
fn edited_point_reduces_only_new_native_ancestor_path() {
    let mut levels = Vec::new();
    for level in 0..4 {
        let mut values = [0; 64];
        values[0] = if level == 0 { 1 } else { 0 };
        values[63] = 1;
        levels.push(vec![
            RawNode {
                values,
                pointers: if level == 0 { 0 } else { 1 | (1 << 63) },
            },
            RawNode {
                values: [3; 64],
                pointers: 0,
            },
        ]);
    }
    let mut source = TestSource {
        versions: vec![vec![1; 2]; 4],
        levels,
        reads: Counter::new(0),
    };
    let mut pyramid = Pyramid::with_capacity(4096);
    let request = region(8, 8, [0; 3]);
    let previous = pyramid.summarize(&source, request, sample).unwrap();
    source.levels[0][0].values[0] = 0;
    for versions in &mut source.versions {
        versions[0] += 1;
    }
    source.reads.set(0);
    let current = pyramid.summarize(&source, request, sample).unwrap();
    assert_eq!(current.occupied() + 1, previous.occupied());
    assert_eq!(source.reads.get(), 4);
}

#[test]
fn malformed_leaf_pointer_is_rejected_without_recursion() {
    let mut source = stripes(0, false);
    source.levels[0][0].pointers = 1;
    let error = Pyramid::new()
        .summarize(&source, region(2, 2, [0; 3]), sample)
        .unwrap_err();
    assert!(matches!(error, Error::Depth));
}
