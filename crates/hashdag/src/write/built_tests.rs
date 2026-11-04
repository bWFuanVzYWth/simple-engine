use super::*;
use crate::{DenseRegion, UniformFill};
use proptest::prelude::*;

fn path(code: u64) -> ZOrder20x3 {
    ZOrder20x3::from_value(code).unwrap()
}

fn audit_references(dag: &HashDag, roots: &[RootId]) -> Result<(), Error> {
    let mut counts: Vec<Vec<usize>> = dag
        .pools
        .iter()
        .map(|pool| vec![0; pool.arrays().0.len()])
        .collect();
    for (level, pool) in dag.pools.iter().enumerate() {
        for slot in 0..pool.arrays().0.len() {
            let index = crate::NodeIdx::new(slot as u64).unwrap();
            if let Some(node) = pool.get(index) {
                for slot in 0..64 {
                    if let Child::Ptr(child) = node.child(slot) {
                        counts[level - 1][child.as_usize()?] += 1;
                    }
                }
            }
        }
    }
    for root in roots {
        if let Child::Ptr(index) = dag.root_child(*root)? {
            counts[DAG_DEPTH as usize - 1][index.as_usize()?] += 1;
        }
    }
    for (pool, counts) in dag.pools.iter().zip(counts) {
        for (slot, count) in counts.into_iter().enumerate() {
            assert_eq!(
                pool.ref_count(crate::NodeIdx::new(slot as u64).unwrap()),
                Some(count)
            );
        }
    }
    Ok(())
}

#[test]
fn odd_even_local_regions_match_independent_morton_values() -> Result<(), Error> {
    let mut builder = LocalBuilder::new();
    let mut local = LocalTree::new();
    let mut dag = HashDag::new()?;
    for lod in 0..=6 {
        let values: Vec<u32> = (0..1_u32 << (3 * lod))
            .map(|code| code.wrapping_mul(0x9e3779b9))
            .collect();
        builder.build(&mut local, lod, &values)?;
        dag.set_built_regions(
            dag.root(),
            &[BuiltRegion {
                path: path(1 << 30),
                tree: &local,
            }],
        )?;
        for index in (0..values.len()).step_by(17).chain([values.len() - 1]) {
            assert_eq!(
                dag.value_at(dag.root(), path((1 << 30) + index as u64))?,
                values[index]
            );
        }
        assert_eq!(dag.value_at(dag.root(), path((1 << 30) - 1))?, 0);
    }
    audit_references(&dag, &[dag.root()])
}

#[test]
fn mixed_scalar_and_odd_regions_preserve_unwritten_leaf_slots() -> Result<(), Error> {
    let mut builder = LocalBuilder::new();
    let mut trees: Vec<_> = (0..4).map(|_| LocalTree::new()).collect();
    let data = [
        vec![u32::MAX],
        (10..18).collect(),
        vec![0],
        (200..712).collect(),
    ];
    let locations = [(1, 0), (8, 1), (63, 0), (512, 3)];
    for ((tree, data), &(_, lod)) in trees.iter_mut().zip(&data).zip(&locations) {
        builder.build(tree, lod, data)?;
    }
    let mut dag = HashDag::new()?;
    dag.fill_uniform(
        dag.root(),
        &[UniformFill {
            path: path(0),
            lod: 4,
            data: 91,
        }],
    )?;
    let regions: Vec<_> = trees
        .iter()
        .zip(&locations)
        .map(|(tree, &(code, _))| BuiltRegion {
            path: path(code),
            tree,
        })
        .collect();
    dag.set_built_regions(dag.root(), &regions)?;
    for code in 0..4096_u64 {
        let expected = data
            .iter()
            .zip(&locations)
            .find_map(|(data, &(base, _))| {
                code.checked_sub(base)
                    .and_then(|offset| data.get(offset as usize))
                    .copied()
            })
            .unwrap_or(91);
        assert_eq!(dag.value_at(dag.root(), path(code))?, expected);
    }
    audit_references(&dag, &[dag.root()])
}

#[test]
fn imported_and_dense_contents_share_one_global_root_and_release_cleanly() -> Result<(), Error> {
    let values: Vec<_> = (0..1_u32 << 18).map(|code| (code & 63) + 1).collect();
    let mut local = LocalTree::new();
    LocalBuilder::new().build(&mut local, 6, &values)?;
    assert_eq!(local.node_count(), 3, "重复空间实例应共享局部节点");
    let mut dag = HashDag::new()?;
    let fork = dag.fork_root(dag.root())?;
    let paths = [path(0), path(1 << 18), path(5 << 18)];
    let dense: Vec<_> = paths
        .iter()
        .map(|&path| DenseRegion {
            path,
            lod: 6,
            values: &values,
        })
        .collect();
    let built: Vec<_> = paths
        .iter()
        .map(|&path| BuiltRegion { path, tree: &local })
        .collect();
    dag.set_dense_regions(dag.root(), &dense)?;
    dag.set_built_regions(fork, &built)?;
    assert_eq!(dag.root_child(dag.root())?, dag.root_child(fork)?);
    audit_references(&dag, &[dag.root(), fork])?;
    dag.release_root(fork)?;
    dag.fill_uniform(
        dag.root(),
        &[UniformFill {
            path: path(0),
            lod: 20,
            data: 0,
        }],
    )?;
    assert!(dag.pools.iter().all(|pool| pool.len() == 0));
    audit_references(&dag, &[dag.root()])
}

#[test]
fn source_reuse_does_not_change_imported_snapshots() -> Result<(), Error> {
    let mut builder = LocalBuilder::new();
    let mut local = LocalTree::new();
    builder.build(&mut local, 4, &(0..4096_u32).collect::<Vec<_>>())?;
    let local_capacity = local.retained_bytes();
    let scratch_capacity = builder.retained_bytes();
    let mut dag = HashDag::new()?;
    dag.set_built_regions(
        dag.root(),
        &[BuiltRegion {
            path: path(0),
            tree: &local,
        }],
    )?;
    let snapshot = dag.fork_root(dag.root())?;
    builder.build(&mut local, 4, &[7; 4096])?;
    assert_eq!(local.retained_bytes(), local_capacity);
    assert_eq!(builder.retained_bytes(), scratch_capacity);
    dag.set_built_regions(
        dag.root(),
        &[BuiltRegion {
            path: path(0),
            tree: &local,
        }],
    )?;
    drop(local);
    assert_eq!(dag.value_at(snapshot, path(193))?, 193);
    assert_eq!(dag.value_at(dag.root(), path(193))?, 7);
    audit_references(&dag, &[dag.root(), snapshot])?;
    dag.release_root(snapshot)?;
    audit_references(&dag, &[dag.root()])
}

#[test]
fn invalid_batch_is_rejected_before_capacity_growth() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let empty = LocalTree::new();
    let before = dag.retained_bytes();
    assert!(matches!(
        dag.set_built_regions(
            dag.root(),
            &[BuiltRegion {
                path: path(0),
                tree: &empty
            }]
        ),
        Err(Error::SubtreeNotBuilt)
    ));
    let mut local = LocalTree::new();
    let mut builder = LocalBuilder::new();
    builder.build(&mut local, 2, &[7; 64])?;
    for paths in [[path(1), path(64)], [path(64), path(0)], [path(0), path(0)]] {
        let regions = paths.map(|path| BuiltRegion { path, tree: &local });
        assert!(matches!(
            dag.set_built_regions(dag.root(), &regions),
            Err(Error::InvalidInput)
        ));
    }
    assert_eq!(dag.retained_bytes(), before);
    assert!(matches!(
        builder.build(&mut local, 2, &[0; 63]),
        Err(Error::InvalidDenseValues)
    ));
    assert_eq!(local.lod(), Some(2));
    Ok(())
}

#[test]
fn late_import_failure_keeps_old_root_and_balanced_references() -> Result<(), Error> {
    let mut builder = LocalBuilder::new();
    let mut good = LocalTree::new();
    let mut bad = LocalTree::new();
    let values: Vec<_> = (0..64_u32).collect();
    builder.build(&mut good, 2, &values)?;
    builder.build(&mut bad, 2, &values)?;
    // 类型之外构造损坏的源，检查事务在已经导入前一地区后仍撤销所有构造持有。
    let mut corrupt = PoolEntry::new(&[0; 64], 1);
    corrupt.set_child(0, Child::Ptr(crate::NodeIdx::ZERO));
    bad.pools[0].clear_local();
    let _ = bad.pools[0].rc_add_or_insert(1, &corrupt)?;
    let mut dag = HashDag::new()?;
    dag.set_cells(
        dag.root(),
        &[crate::SetCell {
            path: path(127),
            data: 42,
        }],
    )?;
    let old = dag.root_child(dag.root())?;
    assert!(matches!(
        dag.set_built_regions(
            dag.root(),
            &[
                BuiltRegion {
                    path: path(0),
                    tree: &good
                },
                BuiltRegion {
                    path: path(64),
                    tree: &bad
                }
            ]
        ),
        Err(Error::NodeMissing(_))
    ));
    assert_eq!(dag.root_child(dag.root())?, old);
    assert_eq!(dag.value_at(dag.root(), path(127))?, 42);
    audit_references(&dag, &[dag.root()])
}

#[test]
fn independently_scheduled_builders_merge_deterministically() -> Result<(), Error> {
    let inputs: Vec<Vec<u32>> = (0..4)
        .map(|chunk| (0..4096).map(|code| code ^ chunk).collect())
        .collect();
    let trees = std::thread::scope(|scope| {
        let workers: Vec<_> = inputs
            .iter()
            .map(|input| {
                scope.spawn(move || {
                    let mut tree = LocalTree::new();
                    LocalBuilder::new().build(&mut tree, 4, input)?;
                    Ok::<_, Error>(tree)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("构建线程不应panic"))
            .collect::<Result<Vec<_>, Error>>()
    })?;
    let mut dag = HashDag::new()?;
    let regions: Vec<_> = trees
        .iter()
        .enumerate()
        .map(|(index, tree)| BuiltRegion {
            path: path((index as u64) << 12),
            tree,
        })
        .collect();
    dag.set_built_regions(dag.root(), &regions)?;
    for (chunk, input) in inputs.iter().enumerate() {
        for index in [0, 63, 64, 4095] {
            assert_eq!(
                dag.value_at(dag.root(), path((chunk as u64) * 4096 + index as u64))?,
                input[index]
            );
        }
    }
    audit_references(&dag, &[dag.root()])
}

proptest! {
    #[test]
    fn arbitrary_u32_imports_match_dense_and_canonical_sharing(
        values in prop::collection::vec(any::<u32>(),512), odd in any::<bool>()
    ) {
        let result=(|| -> Result<(),Error> {
            let lod=if odd {3} else {2};
            let values=&values[..1_usize << (3*lod)];
            let mut local=LocalTree::new();
            LocalBuilder::new().build(&mut local,lod,values)?;
            let mut dag=HashDag::new()?;
            let built=dag.fork_root(dag.root())?;
            dag.set_dense_regions(dag.root(),&[DenseRegion {path:path(0),lod,values}])?;
            dag.set_built_regions(built,&[BuiltRegion {path:path(0),tree:&local}])?;
            assert_eq!(dag.root_child(dag.root())?,dag.root_child(built)?);
            for (code,&value) in values.iter().enumerate() { assert_eq!(dag.value_at(built,path(code as u64))?,value); }
            audit_references(&dag,&[dag.root(),built])?;
            dag.release_root(built)?;
            audit_references(&dag,&[dag.root()])
        })();
        result.map_err(|error|TestCaseError::fail(error.to_string()))?;
    }
}
