use crate::ZOrder20x3;
use proptest::prelude::*;

use super::*;
use crate::{HashDag, SetCell, UniformFill};

fn path(x: u32, y: u32, z: u32) -> ZOrder20x3 {
    crate::test_util::path(x, y, z)
}

fn new_dag() -> HashDag {
    let result = HashDag::new();
    assert!(result.is_ok());
    result.unwrap_or_else(|_| std::process::abort())
}

fn cast<F>(dag: &HashDag, root: RootId, ray: Ray, predicate: F) -> RaycastResult
where
    F: FnMut(RaySample) -> HitBehavior,
{
    dag.raycast(root, ray, predicate).expect("有效 root")
}

fn set_cell(dag: &mut HashDag, x: u32, y: u32, z: u32, value: u32) {
    let cells = [SetCell {
        path: path(x, y, z),
        data: value,
    }];
    let result = dag.set_cells(dag.root(), &cells);
    assert!(result.is_ok(), "{result:?}");
}

fn fill(dag: &mut HashDag, x: u32, y: u32, z: u32, lod: u8, value: u32) {
    let fills = [UniformFill {
        path: path(x, y, z),
        lod,
        data: value,
    }];
    let result = dag.fill_uniform(dag.root(), &fills);
    assert!(result.is_ok(), "{result:?}");
}

const fn ray(origin: [f64; 3], dir: [f64; 3], max_t: f64) -> Ray {
    Ray { origin, dir, max_t }
}

fn assert_state_eq(actual: DdaState, expected: DdaState) {
    assert_eq!(actual, expected);
}

fn state_after_repeated_advance(mut state: DdaState, lod: u8) -> Option<DdaState> {
    let cell = cell_bounds(state.voxel, lod);
    for _ in 0..SAFETY_COUNTER {
        state.advance();
        if !cell_contains(state.voxel, cell) {
            return Some(state);
        }
    }
    None
}

const fn cell_contains(voxel: IVec3, cell: CellBounds) -> bool {
    voxel.x >= cell.min.x
        && voxel.x < cell.max.x
        && voxel.y >= cell.min.y
        && voxel.y < cell.max.y
        && voxel.z >= cell.min.z
        && voxel.z < cell.max.z
}

const fn solid_hit(sample: RaySample) -> HitBehavior {
    if sample.value == 0 {
        HitBehavior::Pass
    } else {
        HitBehavior::Hit
    }
}

const fn target_hit(sample: RaySample) -> HitBehavior {
    if sample.value == 99 {
        HitBehavior::Hit
    } else {
        HitBehavior::Pass
    }
}

#[test]
fn nan_origin_is_invalid() {
    let dag = new_dag();
    let result = cast(
        &dag,
        dag.root(),
        ray([f64::NAN, 0.0, 0.0], [1.0, 0.0, 0.0], 10.0),
        solid_hit,
    );
    assert!(matches!(
        result,
        RaycastResult::InvalidInput {
            reason: InvalidInputReason::OriginNonFinite
        }
    ));
}

#[test]
fn zero_dir_is_invalid() {
    let dag = new_dag();
    let result = cast(
        &dag,
        dag.root(),
        ray([0.0, 0.0, 0.0], [0.0, 0.0, 0.0], 10.0),
        solid_hit,
    );
    assert!(matches!(
        result,
        RaycastResult::InvalidInput {
            reason: InvalidInputReason::DirZero
        }
    ));
}

#[test]
fn negative_reach_is_invalid() {
    let dag = new_dag();
    let result = cast(
        &dag,
        dag.root(),
        ray([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], -1.0),
        solid_hit,
    );
    assert!(matches!(
        result,
        RaycastResult::InvalidInput {
            reason: InvalidInputReason::ReachInvalid
        }
    ));
}

#[test]
fn axis_ray_hits_cell_from_outside() {
    let mut dag = new_dag();
    set_cell(&mut dag, 0, 0, 0, 7);

    let result = cast(
        &dag,
        dag.root(),
        ray([-1.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (0, 0, 0));
    assert_eq!(hit.value, 7);
    assert_eq!(hit.lod, 0);
    assert_eq!(hit.face_axis, Some(Axis::X));
    assert!((hit.t - 1.5).abs() < 1e-9);
    assert_eq!(hit.prev_empty, (-1, 0, 0));
}

#[test]
fn uniform_pass_skips_to_later_hit() {
    let mut dag = new_dag();
    set_cell(&mut dag, 70, 0, 0, 5);
    let mut calls = 0u32;

    let result = cast(
        &dag,
        dag.root(),
        ray([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 100.0),
        |sample| {
            calls += 1;
            solid_hit(sample)
        },
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (70, 0, 0));
    assert!(calls < 20);
}

#[test]
fn predicate_block_returns_blocked() {
    let dag = new_dag();
    let result = cast(
        &dag,
        dag.root(),
        ray([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0),
        |_| HitBehavior::Block,
    );

    assert!(matches!(
        result,
        RaycastResult::Blocked {
            by_voxel: (0, 0, 0),
            value: 0,
            lod: HashDag::AXIS_BITS,
            reason: BlockedReason::PredicateBlock,
            ..
        }
    ));
}

#[test]
fn max_reach_zero_initial_hit_still_reports() {
    let mut dag = new_dag();
    set_cell(&mut dag, 0, 0, 0, 1);

    let result = cast(
        &dag,
        dag.root(),
        ray([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 0.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert!(hit.is_initial());
    assert!(hit.t.abs() < 1e-12);
}

#[test]
fn max_reach_zero_after_pass_misses() {
    let dag = new_dag();
    let result = cast(
        &dag,
        dag.root(),
        ray([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 0.0),
        solid_hit,
    );

    assert!(matches!(
        result,
        RaycastResult::Miss {
            reason: MissReason::ReachExceeded
        }
    ));
}

#[test]
fn leaving_root_returns_boundary_miss() {
    let dag = new_dag();
    let root_edge = f64::from(ROOT_EDGE);
    let result = cast(
        &dag,
        dag.root(),
        ray([root_edge - 0.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0),
        solid_hit,
    );

    assert!(matches!(
        result,
        RaycastResult::Miss {
            reason: MissReason::BoundaryExceeded
        }
    ));
}

#[test]
fn diagonal_tie_breaks_to_x_axis() {
    let mut dag = new_dag();
    set_cell(&mut dag, 1, 0, 0, 7);
    set_cell(&mut dag, 0, 1, 0, 8);

    let result = cast(
        &dag,
        dag.root(),
        ray([0.5, 0.5, 0.5], [1.0, 1.0, 0.0], 10.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (1, 0, 0));
    assert_eq!(hit.value, 7);
    assert_eq!(hit.face_axis, Some(Axis::X));
}

#[test]
fn uniform_nonzero_lod_is_reported() {
    let mut dag = new_dag();
    fill(&mut dag, 64, 0, 0, 6, 9);

    let result = cast(
        &dag,
        dag.root(),
        ray([63.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (64, 0, 0));
    assert_eq!(hit.value, 9);
    assert_eq!(hit.lod, 6);
}

#[test]
fn skip_lod_cell_matches_repeated_advance() {
    let cases = [
        (DVec3::new(2.25, 3.25, 4.25), DVec3::new(1.0, 0.0, 0.0), 2),
        (DVec3::new(7.75, 6.25, 5.25), DVec3::new(-1.0, 0.0, 0.0), 2),
        (DVec3::new(0.5, 0.5, 0.5), DVec3::new(1.0, 1.0, 0.0), 2),
        (DVec3::new(9.5, 8.5, 7.5), DVec3::new(-1.0, 0.0, -1.0), 3),
    ];

    for (origin, dir, lod) in cases {
        let mut skipped = DdaState::init(origin, dir, 0.0, None);
        let repeated = state_after_repeated_advance(skipped, lod);
        assert!(repeated.is_some());
        let Some(repeated) = repeated else {
            return;
        };
        skipped.skip_lod_cell(lod);
        assert_state_eq(skipped, repeated);
    }
}

#[test]
fn completed_reach_wins_over_exhausted_leaf_budget() -> Result<(), Error> {
    let mut dag = HashDag::new()?;
    let cells: Vec<_> = (0..8192)
        .map(|x| SetCell {
            path: path(x, 0, 0),
            data: 1 + (x & 1),
        })
        .collect();
    dag.set_cells(dag.root(), &cells)?;
    let mut callbacks = 0;
    let result = dag.raycast(dag.root(), ray([0.5; 3], [1.0, 0.0, 0.0], 8191.0), |_| {
        callbacks += 1;
        HitBehavior::Pass
    })?;
    assert_eq!(callbacks, 8192);
    assert_eq!(
        result,
        RaycastResult::Miss {
            reason: MissReason::ReachExceeded
        }
    );
    Ok(())
}

#[test]
fn dense_line_scan_hits_target() {
    let mut dag = new_dag();
    for x in 0..64 {
        let value = if x == 63 { 99 } else { 7 };
        set_cell(&mut dag, x, 4, 4, value);
    }

    let result = cast(
        &dag,
        dag.root(),
        ray([-1.5, 4.5, 4.5], [1.0, 0.0, 0.0], 100.0),
        target_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (63, 4, 4));
    assert_eq!(hit.value, 99);
}

#[test]
fn entry_normal_uses_integer_prev_empty() {
    let mut dag = new_dag();
    set_cell(&mut dag, 0, 0, 0, 7);

    let result = cast(
        &dag,
        dag.root(),
        ray([-1.5, 0.5, 0.5], [1.0, 0.0, 0.0], 10.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.entry_normal(), Some((-1, 0, 0)));
}

#[test]
fn root_max_face_entry_hits_last_voxel() {
    let mut dag = new_dag();
    let edge = ROOT_EDGE_U32;
    set_cell(&mut dag, edge - 1, 0, 0, 11);

    let result = cast(
        &dag,
        dag.root(),
        ray([f64::from(edge) + 1.0, 0.5, 0.5], [-1.0, 0.0, 0.0], 4.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (edge - 1, 0, 0));
    assert_eq!(hit.prev_empty, (ROOT_EDGE, 0, 0));
    assert_eq!(hit.entry_normal(), Some((1, 0, 0)));
}

#[test]
fn voxel_face_origin_is_initial_hit() {
    let mut dag = new_dag();
    set_cell(&mut dag, 1, 0, 0, 3);

    let result = cast(
        &dag,
        dag.root(),
        ray([1.0, 0.5, 0.5], [1.0, 0.0, 0.0], 1.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (1, 0, 0));
    assert!(hit.is_initial());
    assert_eq!(hit.entry_normal(), None);
}

#[test]
fn diagonal_uniform_skip_preserves_tie_voxel() {
    let mut dag = new_dag();
    set_cell(&mut dag, 64, 63, 0, 13);
    set_cell(&mut dag, 63, 64, 0, 17);

    let result = cast(
        &dag,
        dag.root(),
        ray([0.5, 0.5, 0.5], [1.0, 1.0, 0.0], 128.0),
        solid_hit,
    );
    assert!(result.as_hit().is_some(), "{result:?}");
    let Some(hit) = result.as_hit() else {
        return;
    };

    assert_eq!(hit.voxel, (64, 63, 0));
    assert_eq!(hit.value, 13);
    assert_eq!(hit.prev_empty, (63, 63, 0));
    assert_eq!(hit.entry_normal(), Some((-1, 0, 0)));
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
            "proptest-regressions/raycast.txt",
        ))),
        ..ProptestConfig::default()
    })]
    #[test]
    fn raycast_is_deterministic(
        ox in 0.0f64..16.0,
        oy in 0.0f64..16.0,
        oz in 0.0f64..16.0,
        dx in -1.0f64..1.0,
        dy in -1.0f64..1.0,
        dz in -1.0f64..1.0,
        reach in 0.0f64..64.0,
    ) {
        let len_sq = dz.mul_add(dz, dy.mul_add(dy, dx * dx));
        prop_assume!(len_sq > 1e-6);

        let mut dag = new_dag();
        set_cell(&mut dag, 8, 8, 8, 1);

        let ray = ray([ox, oy, oz], [dx, dy, dz], reach);
        let a = cast(&dag,dag.root(), ray, solid_hit);
        let b = cast(&dag,dag.root(), ray, solid_hit);
        prop_assert_eq!(a, b);
    }

    #[test]
    fn prev_empty_is_one_step_back_on_x_hit(origin_x in -20.0f64..9.0) {
        let mut dag = new_dag();
        set_cell(&mut dag, 10, 0, 0, 1);

        let result = cast(&dag,dag.root(),
            ray([origin_x, 0.5, 0.5], [1.0, 0.0, 0.0], 64.0),
            solid_hit,
        );
        if let RaycastResult::Hit(hit) = result {
            prop_assert_eq!(hit.voxel, (10, 0, 0));
            prop_assert_eq!(hit.face_axis, Some(Axis::X));
            prop_assert_eq!(hit.prev_empty, (9, 0, 0));
        }
    }
}

#[test]
fn batch_callbacks_finish_each_ray_in_input_order() {
    let mut dag = new_dag();
    set_cell(&mut dag, 3, 0, 0, 7);
    let inputs = [
        ray([0.5; 3], [1.0, 0.0, 0.0], 8.0),
        ray([f64::NAN, 0.5, 0.5], [1.0, 0.0, 0.0], 8.0),
        ray([5.5, 0.5, 0.5], [-1.0, 0.0, 0.0], 8.0),
    ];
    let mut output = [RaycastResult::Miss {
        reason: MissReason::BoundaryExceeded,
    }; 3];
    let mut callbacks = Vec::new();
    dag.raycast_batch(dag.root(), &inputs, &mut output, |index, sample| {
        callbacks.push((index, sample.step_count));
        solid_hit(sample)
    })
    .expect("有效批输入");
    assert!(callbacks.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    for index in [0, 2] {
        let steps: Vec<_> = callbacks
            .iter()
            .filter_map(|&(ray, step)| (ray == index).then_some(step))
            .collect();
        assert!(!steps.is_empty());
        assert_eq!(
            steps,
            (1..=u32::try_from(steps.len()).expect("小批量")).collect::<Vec<_>>()
        );
        assert_eq!(
            output[index],
            cast(&dag, dag.root(), inputs[index], solid_hit)
        );
    }
    assert_eq!(
        output[1],
        RaycastResult::InvalidInput {
            reason: InvalidInputReason::OriginNonFinite
        }
    );
}

#[test]
fn batch_wrong_owner_preserves_output() {
    let dag = new_dag();
    let foreign = new_dag();
    let input = [ray([0.5; 3], [1.0, 0.0, 0.0], 8.0)];
    let sentinel = RaycastResult::Miss {
        reason: MissReason::ReachExceeded,
    };
    let mut output = [sentinel];
    let result = dag.raycast_batch(foreign.root(), &input, &mut output, |_, _| {
        panic!("非法 root 不应回调")
    });
    assert!(matches!(result, Err(Error::WrongOwner)));
    assert_eq!(output, [sentinel]);
}

#[test]
fn batch_released_root_preserves_output() {
    let mut dag = new_dag();
    let released = dag.fork_root(dag.root()).expect("复制 root");
    dag.release_root(released).expect("释放 root");
    let input = [ray([0.5; 3], [1.0, 0.0, 0.0], 8.0)];
    let sentinel = RaycastResult::Miss {
        reason: MissReason::ReachExceeded,
    };
    let mut output = [sentinel];
    let result = dag.raycast_batch(released, &input, &mut output, |_, _| {
        panic!("失效 root 不应回调")
    });
    assert!(matches!(result, Err(Error::RootMissing)));
    assert_eq!(output, [sentinel]);
}

#[test]
fn batch_length_mismatch_preserves_output() {
    let dag = new_dag();
    let sentinel = RaycastResult::Miss {
        reason: MissReason::ReachExceeded,
    };
    let mut output = [sentinel];
    let result = dag.raycast_batch(dag.root(), &[], &mut output, |_, _| {
        panic!("长度错误不应回调")
    });
    assert!(matches!(result, Err(Error::InvalidInput)));
    assert_eq!(output, [sentinel]);
}

#[test]
fn single_wrong_owner_returns_lifecycle_error() {
    let dag = new_dag();
    let foreign = new_dag();
    assert!(matches!(
        dag.raycast(
            foreign.root(),
            ray([0.5; 3], [1.0, 0.0, 0.0], 8.0),
            |_| panic!("非法 root 不应回调")
        ),
        Err(Error::WrongOwner)
    ));
}

#[test]
fn missing_node_reports_corruption() {
    let mut dag = new_dag();
    dag.roots.slots[0].child = Some(Child::Ptr(crate::NodeIdx::ZERO));
    let result = cast(
        &dag,
        dag.root(),
        ray([0.5; 3], [1.0, 0.0, 0.0], 8.0),
        |_| panic!("缺失节点不应回调"),
    );
    assert_eq!(
        result,
        RaycastResult::Aborted {
            reason: AbortReason::DagCorruption
        }
    );
}

// 逐体素浮点 DDA，只读取独立场景函数，不使用池、祖先缓存、准备或跳格内核。
// 代表方向分量为 0 或 ±1，避免以浮点累加误差充当定点 DDA 的边界标准。
fn voxel_oracle(input: Ray) -> Option<(VoxelCoord, Option<Axis>, SignedVoxelCoord, f64)> {
    let length = input
        .dir
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    let dir = input.dir.map(|value| value / length);
    let mut voxel = input.origin.map(|value| value.floor() as i32);
    let step = dir.map(|value| {
        if value > 0.0 {
            1
        } else if value < 0.0 {
            -1
        } else {
            0
        }
    });
    let delta = dir.map(|value| value.abs().recip());
    let mut next = std::array::from_fn::<_, 3, _>(|axis| {
        if step[axis] == 0 {
            return f64::INFINITY;
        }
        let plane = voxel[axis] + i32::from(step[axis] > 0);
        (f64::from(plane) - input.origin[axis]) / dir[axis]
    });
    let mut t = 0.0;
    let mut face = None;
    let mut previous = voxel;
    for _ in 0..256 {
        if t > input.max_t || voxel.iter().any(|&value| !(0..ROOT_EDGE).contains(&value)) {
            return None;
        }
        let coord = voxel.map(|value| u32::try_from(value).expect("root 内"));
        let coord = (coord[0], coord[1], coord[2]);
        if oracle_value(coord) != 0 {
            return Some((coord, face, (previous[0], previous[1], previous[2]), t));
        }
        let axis = if next[0] <= next[1] && next[0] <= next[2] {
            0
        } else if next[1] <= next[2] {
            1
        } else {
            2
        };
        previous = voxel;
        voxel[axis] += step[axis];
        t = next[axis];
        next[axis] += delta[axis];
        face = Some(Axis::from_usize(axis));
    }
    panic!("oracle 代表输入应在有界步数内结束")
}

const fn oracle_value((x, y, z): VoxelCoord) -> u32 {
    if x >= 8 || y >= 8 || z >= 8 || (x * 17 + y * 7 + z * 3) % 5 < 3 {
        0
    } else {
        1 + (x + y * 3 + z * 5) % 13
    }
}

#[test]
fn ray_hits_match_independent_voxel_oracle() {
    let mut dag = new_dag();
    let mut cells = Vec::new();
    for z in 0..8 {
        for y in 0..8 {
            for x in 0..8 {
                let value = oracle_value((x, y, z));
                if value != 0 {
                    cells.push(SetCell {
                        path: path(x, y, z),
                        data: value,
                    });
                }
            }
        }
    }
    cells.sort_unstable_by_key(|cell| cell.path.value());
    dag.set_cells(dag.root(), &cells).expect("独立场景");
    let directions = [
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0],
        [1.0, 1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, -1.0, 1.0],
        [1.0, 1.0, 1.0],
    ];
    for x in [0.25, 2.5, 7.75, 9.5] {
        for y in [0.25, 3.5, 6.75] {
            for z in [0.25, 4.5, 7.75] {
                for dir in directions {
                    for max_t in [0.0, 2.0, 32.0] {
                        let input = ray([x, y, z], dir, max_t);
                        let actual = cast(&dag, dag.root(), input, solid_hit);
                        let expected = voxel_oracle(input);
                        assert!(!actual.is_anomalous(), "{input:?}: {actual:?}");
                        match (actual.as_hit(), expected) {
                            (Some(hit), Some((voxel, face, previous, t))) => {
                                assert_eq!(
                                    (hit.voxel, hit.face_axis, hit.prev_empty),
                                    (voxel, face, previous),
                                    "{input:?}"
                                );
                                assert!((hit.t - t).abs() < 1e-9, "{input:?}: {hit:?}, t={t}");
                            }
                            (None, None) => {}
                            _ => panic!("{input:?}: {actual:?}, oracle={expected:?}"),
                        }
                    }
                }
            }
        }
    }
}
