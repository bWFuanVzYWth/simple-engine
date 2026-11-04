//! 标量 dense、同叶混合区域与快照释放的行为约束。

use crate::{DenseRegion, Error, HashDag, UniformFill, ZOrder20x3};

fn path(code: u64) -> ZOrder20x3 {
    ZOrder20x3::from_value(code).expect("测试坐标")
}

fn scalars(points: &[(u64, u32)]) -> Vec<DenseRegion<'_>> {
    points
        .iter()
        .map(|point| DenseRegion {
            path: path(point.0),
            lod: 0,
            values: std::slice::from_ref(&point.1),
        })
        .collect()
}

#[test]
fn sparse_scalars_preserve_existing_gaps() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    dag.fill_uniform(
        root,
        &[UniformFill {
            path: path(0),
            lod: 20,
            data: 7,
        }],
    )?;
    let points = [(1, 11), (31, 13), (63, 17), (64, 19), (127, 23)];
    dag.set_dense_regions(root, &scalars(&points))?;
    let paths: Vec<_> = (0..129).map(path).collect();
    let mut actual = vec![0; paths.len()];
    dag.values_at(root, &paths, &mut actual)?;
    let mut expected = vec![7; paths.len()];
    for (code, value) in points {
        expected[code as usize] = value;
    }
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn scalar_odd_scalar_sequence_routes_within_one_leaf() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let before = [(0, 11), (7, 13)];
    let after = [(16, 17), (63, 19)];
    let tail = [(128, 23)];
    let odd = [31, 32, 33, 34, 35, 36, 37, 38];
    let even: [u32; 64] = std::array::from_fn(|index| 100 + index as u32);
    let mut regions = scalars(&before);
    regions.push(DenseRegion {
        path: path(8),
        lod: 1,
        values: &odd,
    });
    regions.extend(scalars(&after));
    regions.push(DenseRegion {
        path: path(64),
        lod: 2,
        values: &even,
    });
    regions.extend(scalars(&tail));
    dag.set_dense_regions(root, &regions)?;
    let paths: Vec<_> = (0..129).map(path).collect();
    let mut actual = vec![0; paths.len()];
    dag.values_at(root, &paths, &mut actual)?;
    let mut expected = vec![0; paths.len()];
    expected[0] = 11;
    expected[7] = 13;
    expected[8..16].copy_from_slice(&odd);
    expected[16] = 17;
    expected[63] = 19;
    expected[64..128].copy_from_slice(&even);
    expected[128] = 23;
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn scalar_snapshot_release_reclaims_shared_nodes() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.fork_root(dag.root())?;
    let initial = [
        (0, 3),
        (1, 5),
        (31, 7),
        (63, 11),
        (64, 13),
        (65, 17),
        (127, 19),
        (128, 23),
    ];
    dag.set_dense_regions(root, &scalars(&initial))?;
    let snapshot = dag.fork_root(root)?;
    let edited = [(31, 29), (63, 31), (64, 37)];
    dag.set_dense_regions(root, &scalars(&edited))?;
    dag.release_root(root)?;
    dag.release_root(snapshot)?;
    assert_eq!(crate::test_util::total_len(&dag.pools), 0);
    Ok(())
}
