use super::*;
use crate::{DenseRegion, SetCell, UniformFill, ZOrder20x3};
use proptest::{collection::vec, prelude::*};
use std::collections::HashSet;

fn path(code: u64) -> ZOrder20x3 {
    ZOrder20x3::from_value(code).unwrap()
}

fn reachable(dag: &HashDag, root: RootId) -> Result<HashSet<NodeAddress>, Error> {
    let mut nodes = HashSet::new();
    let mut stack = vec![(HashDag::DEPTH - 1, dag.root_child(root)?)];
    while let Some((level, child)) = stack.pop() {
        let Child::Ptr(index) = child else {
            continue;
        };
        if !nodes.insert(NodeAddress { level, index }) || level == 0 {
            continue;
        }
        for child in dag.children_of(level, index)? {
            stack.push((level - 1, child));
        }
    }
    Ok(nodes)
}

fn assert_difference(
    dag: &HashDag,
    before: RootId,
    after: RootId,
    diff: &[NodeAddress],
) -> Result<(), Error> {
    let old_nodes = reachable(dag, before)?;
    let new_nodes = reachable(dag, after)?;
    let output: HashSet<_> = diff.iter().copied().collect();
    assert!(diff.windows(2).all(|pair| pair[0] < pair[1]));
    let expected: HashSet<_> = new_nodes.difference(&old_nodes).copied().collect();
    assert_eq!(output, expected);
    Ok(())
}

#[test]
fn subtree_reused_from_another_old_position_is_not_uploaded() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    dag.set_cells(
        root,
        &[
            SetCell {
                path: path(0),
                data: 3,
            },
            SetCell {
                path: path(64),
                data: 5,
            },
        ],
    )?;
    let before = dag.fork_root(root)?;
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 5,
        }],
    )?;
    let mut diff = NodeDiff::new();
    assert_difference(&dag, before, root, diff.between(&dag, before, root)?)?;
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn single_point_has_one_changed_node_per_level() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let cell = path(7319);
    dag.set_cells(
        root,
        &[SetCell {
            path: cell,
            data: 7,
        }],
    )?;
    let before = dag.fork_root(root)?;
    dag.set_cells(
        root,
        &[SetCell {
            path: cell,
            data: 9,
        }],
    )?;
    let mut diff = NodeDiff::new();
    let output = diff.between(&dag, before, root)?;
    assert_eq!(output.len(), usize::from(HashDag::DEPTH));
    assert!(output.iter().map(|node| node.level).eq(0..HashDag::DEPTH));
    assert_difference(&dag, before, root, output)?;
    assert_eq!(dag.value_at(before, cell)?, 7);
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn restored_content_has_no_node_changes() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 7,
        }],
    )?;
    let before = dag.fork_root(root)?;
    let mut diff = NodeDiff::new();
    assert!(diff.between(&dag, before, root)?.is_empty());
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 9,
        }],
    )?;
    assert!(!diff.between(&dag, before, root)?.is_empty());
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 7,
        }],
    )?;
    assert_eq!(dag.root_child(before)?, dag.root_child(root)?);
    assert!(diff.between(&dag, before, root)?.is_empty());
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn uniform_root_changes_need_only_root_metadata() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    dag.set_cells(
        root,
        &[SetCell {
            path: path(5),
            data: 3,
        }],
    )?;
    let before = dag.fork_root(root)?;
    dag.fill_uniform(
        root,
        &[UniformFill {
            path: path(0),
            lod: HashDag::AXIS_BITS,
            data: 7,
        }],
    )?;
    let mut diff = NodeDiff::new();
    assert_ne!(dag.root_child(before)?, dag.root_child(root)?);
    assert!(diff.between(&dag, before, root)?.is_empty());
    assert_difference(&dag, before, root, &[])?;
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn repeated_subtrees_are_uploaded_once() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let before = dag.fork_root(root)?;
    let values: Vec<_> = (0..1_u32 << 18).map(|code| code & 63).collect();
    dag.set_dense_regions(
        root,
        &[DenseRegion {
            path: path(0),
            lod: 6,
            values: &values,
        }],
    )?;
    let mut diff = NodeDiff::new();
    let output = diff.between(&dag, before, root)?;
    assert_eq!(output.len(), usize::from(HashDag::DEPTH));
    assert_eq!(output.iter().filter(|node| node.level == 0).count(), 1);
    assert_difference(&dag, before, root, output)?;
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn shared_nodes_with_different_old_neighbors_have_exact_difference() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let old: Vec<_> = (0..4096_u32)
        .map(|code| (code & 63) + (code >> 6) % 3)
        .collect();
    dag.set_dense_regions(
        root,
        &[DenseRegion {
            path: path(0),
            lod: 4,
            values: &old,
        }],
    )?;
    let before = dag.fork_root(root)?;
    let new: Vec<_> = (0..4096_u32)
        .map(|code| (code & 63) + (code >> 6) % 2)
        .collect();
    dag.set_dense_regions(
        root,
        &[DenseRegion {
            path: path(0),
            lod: 4,
            values: &new,
        }],
    )?;
    let mut diff = NodeDiff::new();
    assert_difference(&dag, before, root, diff.between(&dag, before, root)?)?;
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn both_root_owners_are_checked() -> Result<(), Error> {
    let dag = HashDag::new()?;
    let other = HashDag::new()?;
    let mut diff = NodeDiff::new();
    assert!(matches!(
        diff.between(&dag, other.root(), dag.root()),
        Err(Error::WrongOwner)
    ));
    assert!(matches!(
        diff.between(&dag, dag.root(), other.root()),
        Err(Error::WrongOwner)
    ));
    Ok(())
}

#[test]
fn failed_diff_clears_output_and_allows_reuse() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let before = dag.fork_root(root)?;
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 7,
        }],
    )?;
    let mut diff = NodeDiff::new();
    let expected = diff.between(&dag, before, root)?.to_vec();
    assert!(!expected.is_empty());
    let other = HashDag::new()?;
    assert!(matches!(
        diff.between(&dag, before, other.root()),
        Err(Error::WrongOwner)
    ));
    assert!(diff.nodes().is_empty());
    assert_eq!(diff.between(&dag, before, root)?, expected);
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn released_root_is_rejected_after_slot_reuse() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let before = dag.fork_root(root)?;
    dag.release_root(before)?;
    let replacement = dag.fork_root(root)?;
    let mut diff = NodeDiff::new();
    assert!(matches!(
        diff.between(&dag, before, root),
        Err(Error::RootMissing)
    ));
    assert!(matches!(
        diff.between(&dag, root, before),
        Err(Error::RootMissing)
    ));
    assert!(diff.between(&dag, replacement, root)?.is_empty());
    dag.release_root(replacement)?;
    Ok(())
}

#[test]
fn reused_node_slots_are_compared_again() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let root = dag.root();
    let before = dag.fork_root(root)?;
    let mut diff = NodeDiff::new();
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 7,
        }],
    )?;
    diff.between(&dag, before, root)?;
    let first = dag.root_child(root)?;
    dag.set_cells(
        root,
        &[SetCell {
            path: path(0),
            data: 0,
        }],
    )?;
    dag.set_cells(
        root,
        &[
            SetCell {
                path: path(0),
                data: 9,
            },
            SetCell {
                path: path(64),
                data: 11,
            },
        ],
    )?;
    assert_eq!(dag.root_child(root)?, first);
    assert_difference(&dag, before, root, diff.between(&dag, before, root)?)?;
    dag.release_root(before)?;
    Ok(())
}

#[test]
fn buffers_can_be_reused_across_dag_owners() -> Result<(), Error> {
    let mut diff = NodeDiff::new();
    for cells in [
        vec![SetCell {
            path: path(0),
            data: 7,
        }],
        vec![
            SetCell {
                path: path(0),
                data: 7,
            },
            SetCell {
                path: path(64),
                data: 9,
            },
        ],
    ] {
        let mut dag = HashDag::new()?;
        let root = dag.root();
        let before = dag.fork_root(root)?;
        dag.set_cells(root, &cells)?;
        assert_difference(&dag, before, root, diff.between(&dag, before, root)?)?;
        dag.release_root(before)?;
    }
    Ok(())
}

proptest! {
    #[test]
    fn successive_edits_preserve_exact_difference(
        batches in vec(vec((0_u16..4096, 0_u8..6), 0..12), 1..20),
    ) {
        let checked = |error: Error| TestCaseError::fail(error.to_string());
        let mut dag = HashDag::new().map_err(checked)?;
        let root = dag.root();
        let mut diff = NodeDiff::new();
        for batch in batches {
            let before = dag.fork_root(root).map_err(checked)?;
            let mut cells: Vec<_> = batch.into_iter().map(|(code, data)| SetCell {
                path: path(u64::from(code)), data: u32::from(data),
            }).collect();
            cells.sort_by_key(|cell| cell.path.value());
            dag.set_cells(root, &cells).map_err(checked)?;
            assert_difference(&dag, before, root, diff.between(&dag, before, root).map_err(checked)?)
                .map_err(checked)?;
            dag.release_root(before).map_err(checked)?;
        }
    }
}
