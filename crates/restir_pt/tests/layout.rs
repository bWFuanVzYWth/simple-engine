//! NV scalar ABI、Morton索引与阶段调度边界。
use restir_pt::{
    Dispatch, Error, Fusion, HitInfo, Layout, NeighborMask, PathReservoir, PrecomputedShift,
    Reconnection, Stage, WorkItem,
};
use std::mem::{align_of, offset_of, size_of};

#[test]
fn scalar_abi_matches_uncompressed_nv_buffers() {
    assert_eq!(size_of::<HitInfo>(), 16);
    assert_eq!(size_of::<PathReservoir>(), 80);
    assert_eq!(align_of::<PathReservoir>(), 4);
    assert_eq!(offset_of!(PathReservoir, reconnection_hit), 36);
    assert_eq!(offset_of!(PathReservoir, reconnection_jacobian), 52);
    assert_eq!(offset_of!(PathReservoir, reconnection_wi), 68);
    assert_eq!(size_of::<Reconnection>(), 60);
    assert_eq!(offset_of!(Reconnection, previous_wo), 28);
    assert_eq!(offset_of!(Reconnection, previous_incident), 44);
    assert_eq!(size_of::<PrecomputedShift>(), 20);
    assert_eq!(size_of::<NeighborMask>(), 8);
}

#[test]
fn morton_tiles_do_not_alias_on_partial_edges() {
    let layout = Layout::new([17, 19]).unwrap();
    let mut occupied = vec![false; layout.elements() as usize];
    for y in 0..19 {
        for x in 0..17 {
            let index = layout.reservoir_offset([x, y]).unwrap() as usize;
            assert!(!occupied[index]);
            occupied[index] = true;
        }
    }
    assert_eq!(occupied.into_iter().filter(|v| *v).count(), 17 * 19);
    assert_eq!(layout.reservoir_offset([16, 0]), Some(256));
    assert_eq!(layout.reservoir_offset([0, 16]), Some(512));
    assert_eq!(layout.reservoir_offset([17, 0]), None);
}

#[test]
fn working_set_counts_padding_and_both_histories() {
    let layout = Layout::new([17, 19]).unwrap();
    assert_eq!(layout.pixels(), 323);
    assert_eq!(layout.elements(), 1024);
    assert_eq!(layout.queue_capacity(), 3072);
    let memory = layout.memory();
    let allocations = [
        memory.reservoir,
        memory.reservoir,
        memory.primary_hit,
        memory.primary_hit,
        memory.color_estimate,
        memory.color_estimate,
        memory.reconnection,
        memory.reconnection_offsets,
        memory.neighbor_mask,
        memory.work_items,
        memory.precomputed,
        memory.counter,
    ];
    assert_eq!(allocations.into_iter().sum::<u64>(), memory.total);
    assert_eq!(memory.total, 464 * u64::from(layout.elements()) + 4);
}

#[test]
fn color_estimates_add_two_padded_rgb_buffers() {
    let base = Layout::with_fusion([17, 19], Fusion::ALL).unwrap();
    let layout = Layout::with_color_estimates([17, 19], Fusion::ALL).unwrap();
    let n = u64::from(layout.elements());
    assert!(!base.has_color_estimates());
    assert_eq!(base.memory().color_estimate, 0);
    assert!(layout.has_color_estimates());
    assert_eq!(layout.memory().color_estimate, 12 * n);
    assert_eq!(layout.memory().total - base.memory().total, 24 * n);
    assert_eq!(layout.fusion(), base.fusion());
}

#[test]
fn fused_working_set_keeps_only_cross_pixel_state() {
    let layout = Layout::with_fusion([17, 19], Fusion::ALL).unwrap();
    let memory = layout.memory();
    assert_eq!(layout.queue_capacity(), 0);
    assert_eq!(
        [
            memory.reconnection,
            memory.reconnection_offsets,
            memory.work_items,
            memory.counter
        ],
        [0; 4]
    );
    assert_eq!(memory.total, 260 * u64::from(layout.elements()));
    assert_eq!(layout.fusion(), Fusion::ALL);
}

#[test]
fn partial_fusion_retains_storage_for_unfused_replay() {
    let fusion = Fusion {
        temporal: true,
        ..Fusion::NONE
    };
    let layout = Layout::with_fusion([17, 19], fusion).unwrap();
    assert_eq!(layout.memory(), Layout::new([17, 19]).unwrap().memory());
}

#[test]
fn temporal_split_rejects_simultaneous_whole_pixel_fusion() {
    let fusion = Fusion {
        temporal: true,
        temporal_split: true,
        ..Fusion::NONE
    };
    assert!(matches!(
        Layout::with_fusion([17, 19], fusion),
        Err(Error::TemporalFusion)
    ));
}

#[test]
fn temporal_split_reuses_existing_paired_storage() {
    let fusion = Fusion {
        temporal: false,
        temporal_split: true,
        ..Fusion::ALL
    };
    let layout = Layout::with_fusion([17, 19], fusion).unwrap();
    assert_eq!(
        layout.memory(),
        Layout::with_fusion([17, 19], Fusion::ALL).unwrap().memory()
    );
}

#[test]
fn temporal_split_keeps_storage_for_unfused_spatial_replay() {
    let fusion = Fusion {
        temporal_split: true,
        ..Fusion::ENDS
    };
    let layout = Layout::with_fusion([17, 19], fusion).unwrap();
    assert_eq!(layout.memory(), Layout::new([17, 19]).unwrap().memory());
}

#[test]
fn temporal_split_dispatches_two_ray_directions_then_compute() {
    let fusion = Fusion {
        temporal_split: true,
        ..Fusion::ENDS
    };
    let layout = Layout::with_fusion([17, 19], fusion).unwrap();
    assert_eq!(
        layout.dispatch(Stage::TemporalReplay),
        Dispatch::Rays([17, 19, 2])
    );
    assert_eq!(
        layout.dispatch(Stage::TemporalMerge),
        Dispatch::Compute([2, 2, 1])
    );
}

#[test]
fn temporal_split_skips_compact_work_and_preserves_barrier_order() {
    let fusion = Fusion {
        temporal_split: true,
        ..Fusion::ENDS
    };
    let warm: Vec<_> = Stage::sequence(true)
        .filter(|stage| stage.enabled(true, fusion))
        .collect();
    assert_eq!(
        warm,
        [
            Stage::Initial,
            Stage::TemporalReplay,
            Stage::TemporalMerge,
            Stage::SpatialWork,
            Stage::SpatialReplay,
            Stage::SpatialPrecompute,
            Stage::SpatialMerge,
        ]
    );
}

#[test]
fn fused_schedule_preserves_only_spatial_global_dependencies() {
    let warm: Vec<_> = Stage::sequence(true)
        .filter(|stage| stage.enabled(true, Fusion::ALL))
        .collect();
    assert_eq!(
        warm,
        [
            Stage::Initial,
            Stage::TemporalMerge,
            Stage::SpatialWork,
            Stage::SpatialPrecompute,
            Stage::SpatialMerge
        ]
    );
    let cold: Vec<_> = Stage::sequence(true)
        .filter(|stage| stage.enabled(false, Fusion::ALL))
        .collect();
    assert_eq!(
        cold,
        [
            Stage::Initial,
            Stage::SpatialWork,
            Stage::SpatialPrecompute,
            Stage::SpatialMerge
        ]
    );
}

#[test]
fn largest_supported_image_keeps_64_bit_byte_totals() {
    let layout = Layout::new([4096, 4096]).unwrap();
    assert_eq!(layout.elements(), 4096 * 4096);
    assert!(layout.memory().total > u64::from(u32::MAX));
}

#[test]
fn largest_compact_buffer_fits_32_bit_addressing() {
    let layout = Layout::new([4096, 4096]).unwrap();
    assert!(layout.memory().reconnection <= u64::from(u32::MAX));
}

#[test]
fn largest_directional_launch_keeps_all_slots() {
    let layout = Layout::with_fusion([4096, 4096], Fusion::ALL).unwrap();
    assert_eq!(
        layout.dispatch(Stage::SpatialPrecompute),
        Dispatch::Rays([12288, 4096, 1])
    );
}

#[test]
fn layout_rejects_coordinates_outside_work_item_domain() {
    assert!(matches!(Layout::new([4097, 1]), Err(Error::Extent)));
    assert!(matches!(Layout::new([1, 0]), Err(Error::Extent)));
}

#[test]
fn work_item_retains_all_coordinate_and_direction_bits() {
    let work = WorkItem::new([4095, 4095], 5).unwrap();
    assert_eq!(work.bits(), 0x05ff_ffff);
    assert_eq!(work.pixel(), [4095, 4095]);
    assert_eq!(work.direction(), 5);
}

#[test]
fn work_item_rejects_truncation() {
    assert!(WorkItem::new([4096, 0], 0).is_err());
    assert!(WorkItem::new([0, 4096], 0).is_err());
    assert!(WorkItem::new([0, 0], 6).is_err());
}

#[test]
fn dispatch_distinguishes_groups_and_ray_invocations() {
    let layout = Layout::new([17, 19]).unwrap();
    assert_eq!(layout.dispatch(Stage::Primary), Dispatch::Rays([17, 19, 1]));
    assert_eq!(
        layout.dispatch(Stage::TemporalReplay),
        Dispatch::Rays([34, 19, 1])
    );
    assert_eq!(
        layout.dispatch(Stage::SpatialReplay),
        Dispatch::Rays([51, 19, 1])
    );
    assert_eq!(
        layout.dispatch(Stage::SpatialWork),
        Dispatch::Compute([2, 2, 1])
    );
    assert_eq!(
        layout.dispatch(Stage::SpatialMerge),
        Dispatch::Compute([2, 2, 1])
    );
}

#[test]
fn invalid_history_skips_only_temporal_stages() {
    let cold: Vec<_> = Stage::sequence(false).collect();
    assert_eq!(
        cold,
        [
            Stage::Primary,
            Stage::Initial,
            Stage::SpatialWork,
            Stage::SpatialReplay,
            Stage::SpatialPrecompute,
            Stage::SpatialMerge,
            Stage::Resolve
        ]
    );
    let warm: Vec<_> = Stage::sequence(true).collect();
    assert_eq!(
        &warm[..5],
        &[
            Stage::Primary,
            Stage::Initial,
            Stage::TemporalWork,
            Stage::TemporalReplay,
            Stage::TemporalMerge
        ]
    );
    assert_eq!(&warm[5..], &cold[2..]);
}
