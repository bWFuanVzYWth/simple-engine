use super::*;
#[test]
fn bird_order_matches_level_one_vulkan_vertex_order() {
    let actual: Vec<_> = MicroTriangles::new(1).unwrap().collect();
    assert_eq!(
        actual,
        vec![
            [[0, 0], [1, 0], [0, 1]],
            [[0, 1], [1, 1], [1, 0]],
            [[1, 0], [2, 0], [1, 1]],
            [[1, 1], [0, 1], [0, 2]]
        ]
    );
}
#[test]
fn aligned_texture_encodes_coverage_and_threshold_in_bird_order() {
    let bytes = [0, 0, 0, 127, 0, 0, 0, 128, 0, 0, 0, 255, 0, 0, 0, 0];
    let t = Texture {
        extent: [2, 2],
        rgba: &bytes,
        bounds: [0, 0, 2, 2],
        cutoff: 128,
    };
    let map = aligned(&t, [[0., 0.], [1., 0.], [0., 1.]], 10)
        .unwrap()
        .unwrap();
    assert_eq!(
        map,
        Map {
            level: 1,
            data: vec![12]
        }
    );
}
#[test]
fn shifted_uv_rejects_inexact_texel_alignment() {
    let t = Texture {
        extent: [4, 4],
        rgba: &[255; 64],
        bounds: [0, 0, 4, 4],
        cutoff: 128,
    };
    assert!(
        aligned(&t, [[0.01, 0.], [1., 0.], [0., 1.]], 10)
            .unwrap()
            .is_none()
    );
}

#[test]
fn finite_uv_outside_atlas_preserves_software_clamp() {
    let t = Texture {
        extent: [1, 1],
        rgba: &[255; 4],
        bounds: [0, 0, 1, 1],
        cutoff: 128,
    };
    assert!(
        aligned(&t, [[-1., 0.], [1., 0.], [0., 2.]], 10)
            .unwrap()
            .is_none()
    );
}
#[test]
fn integer_vertices_do_not_admit_skewed_microtriangles_crossing_texels() {
    let t = Texture {
        extent: [4, 4],
        rgba: &[255; 64],
        bounds: [0, 0, 4, 4],
        cutoff: 128,
    };
    assert!(
        aligned(&t, [[0., 0.], [1., 0.], [0.25, 1.]], 10)
            .unwrap()
            .is_none()
    );
}
#[test]
fn sprite_clamp_matches_nearest_texel_coverage() {
    let mut rgba = [0; 64];
    rgba[23] = 255;
    let t = Texture {
        extent: [4, 4],
        rgba: &rgba,
        bounds: [1, 1, 2, 2],
        cutoff: 128,
    };
    let map = aligned(&t, [[0., 0.], [1., 0.], [0., 1.]], 10)
        .unwrap()
        .unwrap();
    assert!(map.data.into_iter().all(|v| v == 255));
}
#[test]
fn device_level_limit_preserves_software_fallback() {
    let t = Texture {
        extent: [4, 4],
        rgba: &[255; 64],
        bounds: [0, 0, 4, 4],
        cutoff: 128,
    };
    assert!(
        aligned(&t, [[0., 0.], [1., 0.], [0., 1.]], 1)
            .unwrap()
            .is_none()
    );
}
