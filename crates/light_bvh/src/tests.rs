use super::*;

fn quad(x: f32, power: f32) -> Quad {
    Quad {
        vertices: [
            [x, 0.0, 0.0],
            [x + 1.0, 0.0, 0.0],
            [x + 1.0, 1.0, 0.0],
            [x, 1.0, 0.0],
        ],
        power,
    }
}

#[derive(Clone, Copy, Debug)]
struct Child {
    center: [f32; 3],
    power: f32,
    child: u32,
}

fn children(tree: &Tree, packet: u32) -> [Child; 2] {
    let packet = packet as usize;
    let block = packet / 4;
    let within = packet % 4;
    let count = (tree.len() as usize - 1 - block * 4).min(4);
    let words = &tree.words()[block * 40..];
    std::array::from_fn(|side| Child {
        center: std::array::from_fn(|axis| f32::from_bits(words[within * 8 + side * 4 + axis])),
        power: f32::from_bits(words[within * 8 + side * 4 + 3]),
        child: words[count * 8 + within * 2 + side],
    })
}

fn leaf(tree: &Tree, light: usize) -> [u32; 8] {
    let hot_words = (tree.len() as usize - 1) * 10;
    let begin = hot_words.div_ceil(8) * 8 + light * 8;
    tree.words()[begin..begin + 8]
        .try_into()
        .expect("完整叶记录")
}

#[test]
fn empty_and_single_light_need_no_internal_branch() -> Result<(), Error> {
    let empty = Tree::build(&[])?;
    assert!(empty.is_empty());
    assert_eq!(empty.node_count(), 0);
    assert!(empty.words().is_empty());
    let tree = Tree::build(&[quad(2.0, 3.0)])?;
    assert_eq!((tree.len(), tree.node_count()), (1, 1));
    let words = tree.words();
    assert_eq!(words.len(), 8);
    assert_eq!(leaf(&tree, 0)[7], 0);
    assert_eq!(f32::from_bits(leaf(&tree, 0)[3]), 2.0);
    Ok(())
}

#[test]
fn topology_and_paths_cover_each_input_exactly_once() -> Result<(), Error> {
    for l in [2, 3, 4, 5, 6, 7, 8, 9, 127, 128] {
        let quads: Vec<_> = (0..l)
            .map(|i| quad(((i * 73) % l) as f32, (i + 1) as f32))
            .collect();
        let tree = Tree::build(&quads)?;
        let packets = l - 1;
        let mut parents = vec![0; packets];
        let mut seen = vec![false; l];
        let mut counts = vec![0; packets];
        counts[0] = l;
        for at in 0..packets {
            for (side, child) in children(&tree, at as u32).into_iter().enumerate() {
                let count = if side == 0 {
                    counts[at] / 2
                } else {
                    counts[at] - counts[at] / 2
                };
                if child.child & LEAF != 0 {
                    let light = (child.child & !LEAF) as usize;
                    assert!(!seen[light]);
                    seen[light] = true;
                    let path = leaf(&tree, light)[7];
                    assert!(path >> 27 <= l.ilog2() + 1);
                    let mut replay = 0;
                    for depth in 0..path >> 27 {
                        assert_eq!(replay & LEAF, 0);
                        replay = children(&tree, replay)[((path >> depth) & 1) as usize].child;
                    }
                    assert_eq!(replay, child.child);
                    assert_eq!(child.power, quads[light].power);
                    assert_eq!(count, 1);
                } else {
                    let next = child.child as usize;
                    assert!(next > at && next < packets);
                    parents[next] += 1;
                    counts[next] = count;
                    let [left, right] = children(&tree, child.child);
                    assert!(count > 1);
                    assert_eq!(child.power, left.power + right.power);
                }
            }
        }
        assert_eq!(parents[0], 0);
        assert!(parents[1..].iter().all(|&count| count == 1));
        assert!(seen.into_iter().all(|value| value));
    }
    Ok(())
}

#[test]
fn centroid_uses_triangle_area_then_light_power() -> Result<(), Error> {
    let quads = [
        Quad {
            vertices: [[0., 0., 0.], [3., 0., 0.], [0., 3., 0.], [-9., 0., 0.]],
            power: 1.0,
        },
        Quad {
            vertices: [[20., 0., 0.], [23., 0., 0.], [20., 3., 0.], [20., 3., 0.]],
            power: 3.0,
        },
        quad(100.0, 1.0),
        quad(110.0, 1.0),
    ];
    let tree = Tree::build(&quads)?;
    // 第一个 quad 两半面积 4.5、13.5，质心 x=-2；第二个只有一半，质心 x=21。
    let subtree = children(&tree, 0)
        .into_iter()
        .find(|child| {
            child.child & LEAF == 0
                && children(&tree, child.child).map(|leaf| leaf.child) == [LEAF, LEAF | 1]
        })
        .expect("前两个灯形成一个子树");
    let center = (-2.0_f64 + 3.0 * 21.0) / 4.0;
    assert_eq!(subtree.center, [center as f32, 1.0, 0.0]);
    assert_eq!(subtree.power, 4.0);
    Ok(())
}

#[test]
fn equal_spatial_keys_have_deterministic_distinct_identities() -> Result<(), Error> {
    let quads = vec![quad(2.0, 1.0); 257];
    let first = Tree::build(&quads)?;
    let second = Tree::build(&quads)?;
    assert_eq!(first.words(), second.words());
    let paths: Vec<_> = (0..quads.len()).map(|id| leaf(&first, id)[7]).collect();
    for left in 0..paths.len() {
        assert!(!paths[left + 1..].contains(&paths[left]));
    }
    Ok(())
}

#[test]
fn leaf_bounds_include_every_quad_corner() -> Result<(), Error> {
    let tree = Tree::build(&[Quad {
        vertices: [[2., 1., -3.], [5., 2., 0.], [1., 7., 2.], [-4., 3., -1.]],
        power: 1.0,
    }])?;
    let record = leaf(&tree, 0);
    let bounds = [0, 1, 2, 4, 5, 6].map(|at| f32::from_bits(record[at]));
    assert_eq!(bounds, [-4., 1., -3., 5., 7., 2.]);
    assert_eq!(f32::from_bits(record[3]), 63.0);
    Ok(())
}

#[test]
fn encoded_records_preserve_the_documented_buffer_alignment() -> Result<(), Error> {
    for lights in 1..=18 {
        let tree = Tree::build(&vec![quad(2.0, 1.0); lights])?;
        let hot_bytes = (lights - 1) * 40;
        let leaf_bytes = hot_bytes.div_ceil(32) * 32;
        assert_eq!(tree.words().len() * 4, leaf_bytes + lights * 32);
        assert_eq!(tree.words().len() % 8, 0);
        assert!(
            tree.words()[hot_bytes / 4..leaf_bytes / 4]
                .iter()
                .all(|&x| x == 0)
        );
        for light in 0..lights {
            assert_eq!(leaf(&tree, light)[0], 2.0_f32.to_bits());
            assert_eq!(leaf(&tree, light)[4], 3.0_f32.to_bits());
        }
    }
    Ok(())
}

#[test]
fn invalid_quad_is_reported_before_tree_publication() {
    for invalid in [
        Quad {
            power: 0.0,
            ..quad(0.0, 1.0)
        },
        Quad {
            power: f32::NAN,
            ..quad(0.0, 1.0)
        },
        Quad {
            vertices: [[0.0; 3]; 4],
            power: 1.0,
        },
        Quad {
            vertices: [[f32::INFINITY; 3]; 4],
            power: 1.0,
        },
    ] {
        assert!(matches!(
            Tree::build(&[quad(0.0, 1.0), invalid]),
            Err(Error::Quad(1))
        ));
    }
}

#[test]
fn aggregate_power_overflow_is_an_error() {
    assert!(matches!(
        Tree::build(&[quad(0.0, f32::MAX), quad(4.0, f32::MAX)]),
        Err(Error::Power)
    ));
}

#[test]
fn median_partition_matches_independent_total_order() {
    // 倒序、相同空间中心与随机顺序分别覆盖分区退化和 tie-break；oracle 不调用排序内核。
    for length in [2, 17, 127, 512] {
        let mut state = 1_u32;
        let keys: Vec<_> = (0..length)
            .map(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state % 31
            })
            .collect();
        let compare = |a: u32, b: u32| keys[a as usize].cmp(&keys[b as usize]).then(a.cmp(&b));
        let mut order: Vec<_> = (0..length as u32).rev().collect();
        select_median(&mut order, length / 2, &compare);
        let pivot = order[length / 2];
        assert_eq!(
            (0..length as u32)
                .filter(|&id| compare(id, pivot).is_lt())
                .count(),
            length / 2
        );
        assert!(
            order[..length / 2]
                .iter()
                .all(|&id| compare(id, pivot).is_lt())
        );
        assert!(
            order[length / 2 + 1..]
                .iter()
                .all(|&id| compare(id, pivot).is_gt())
        );
    }
}
