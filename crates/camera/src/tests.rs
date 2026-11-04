use super::*;

fn frame(origin: [f64; 3]) -> Frame {
    Frame::from_scaled(origin, [0., 0., -2.], [3., 0., 0.], [0., 1.5, 0.]).unwrap()
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 3e-4, "{a} != {b}");
}

#[test]
fn scaled_basis_keeps_the_traced_frustum() {
    let source = [[0., 0., -2.], [3., 0., 0.], [0., 1.5, 0.]];
    let camera = frame([0.; 3]).relative([0.; 3]).unwrap();
    for uv in [[0.5, 0.5], [0.15, 0.7], [1., 0.]] {
        let source: [f32; 3] = std::array::from_fn(|i| {
            source[0][i] + (uv[0] * 2. - 1.) * source[1][i] - (uv[1] * 2. - 1.) * source[2][i]
        });
        let length = dot(source, source).sqrt();
        for (actual, expected) in camera.direction(uv).unwrap().into_iter().zip(source) {
            close(actual, expected / length);
        }
    }
}

#[test]
fn invalid_camera_inputs_fail_at_construction() {
    for (forward, right, up) in [
        ([0.; 3], [1., 0., 0.], [0., 1., 0.]),
        ([0., 0., -1.], [1., 0., 0.1], [0., 1., 0.]),
        ([0., 0., f32::NAN], [1., 0., 0.], [0., 1., 0.]),
    ] {
        assert_eq!(
            Frame::from_scaled([0.; 3], forward, right, up),
            Err(Error::Basis)
        );
    }
    assert_eq!(frame([0.; 3]).relative([f64::NAN; 3]), Err(Error::Position));
}

#[test]
fn large_world_fraction_is_preserved_before_rounding() {
    let origin = [30_000_000.125, -30_000_000.375, 30_000_000.75];
    let camera = frame(origin)
        .relative([30_000_000., -30_000_000., 30_000_000.])
        .unwrap();
    assert_eq!(camera.position(), [0.125, -0.375, 0.75]);
}

#[test]
fn changing_anchor_keeps_world_projection_and_camera_motion() {
    let old = frame([30_000_000.125, 7., 30_000_000.75]);
    let current = frame([30_000_001.375, 7., 30_000_000.75]);
    let point = [30_000_003.625, 8.25, 29_999_992.5];
    let extent = Extent::new(961, 541).unwrap();
    let mut projections = Vec::new();
    for anchor in [
        [30_000_000., 0., 30_000_000.],
        [30_000_064., 0., 29_999_936.],
    ] {
        let relative = std::array::from_fn(|i| (point[i] - anchor[i]) as f32);
        let previous = old
            .relative(anchor)
            .unwrap()
            .project(relative, extent)
            .unwrap();
        let now = current
            .relative(anchor)
            .unwrap()
            .project(relative, extent)
            .unwrap();
        projections.push((previous.pixel, now.pixel));
    }
    assert_eq!(projections[0], projections[1]);
    assert_ne!(projections[0].0, projections[0].1);
}

#[test]
fn odd_input_extent_keeps_output_frustum_and_center_jitter() {
    let camera = frame([3., 7., 2.]).relative([0.; 3]).unwrap();
    let extent = Extent::new(961, 541).unwrap();
    let pixel = [743, 310];
    let jitter = [0.25, -0.125];
    let ray = camera.ray(pixel, extent, jitter).unwrap();
    let point = std::array::from_fn(|i| ray.origin[i] + ray.direction[i] * 11.);
    let projected = camera.project(point, extent).unwrap();
    for i in 0..2 {
        close(projected.pixel[i], pixel[i] as f32 + 0.5 + jitter[i]);
    }
    assert_eq!(camera.aspect(), 2.0);
    assert_ne!(camera.aspect(), extent.aspect());
}

#[test]
fn row_projection_round_trips_and_maps_positive_depth() {
    let camera = frame([3., 7., 2.]).relative([0.; 3]).unwrap();
    let projection = Projection::new(camera.tangent(), camera.aspect(), 0.01, 1000.).unwrap();
    assert_eq!(
        camera.world_to_view().transform([4., 9., -5., 1.]),
        [1., 2., 7., 1.]
    );
    for matrix in [
        projection.view_to_clip() * projection.clip_to_view(),
        camera.world_to_view() * camera.view_to_world(),
    ] {
        for (actual, expected) in matrix
            .to_array()
            .into_iter()
            .zip(Matrix::IDENTITY.to_array())
        {
            close(actual, expected);
        }
    }
    for (depth, expected) in [(projection.near(), 0.), (projection.far(), 1.)] {
        let clip = projection.view_to_clip().transform([0., 0., depth, 1.]);
        close(clip[2] / clip[3], expected);
    }
}

#[test]
fn clip_transform_accounts_for_translation_rotation_and_fov() {
    let previous = frame([3., 7., 2.]).relative([0.; 3]).unwrap();
    let current = Frame::from_scaled(
        [3.25, 7., 2.],
        [0.6, 0., -0.8],
        [1.04, 0., 0.78],
        [0., 0.7, 0.],
    )
    .unwrap()
    .relative([0.; 3])
    .unwrap();
    let old_projection =
        Projection::new(previous.tangent(), previous.aspect(), 0.01, 1000.).unwrap();
    let projection = Projection::new(current.tangent(), current.aspect(), 0.01, 1000.).unwrap();
    let point = [4., 9., -5., 1.];
    let clip = (current.world_to_view() * projection.view_to_clip()).transform(point);
    let actual = current
        .clip_to(projection, previous, old_projection)
        .transform(clip);
    let expected = (previous.world_to_view() * old_projection.view_to_clip()).transform(point);
    for i in 0..3 {
        close(actual[i] / actual[3], expected[i] / expected[3]);
    }
}

#[test]
fn invalid_depth_ranges_and_sampling_are_rejected() {
    for (near, far) in [
        (0., 100.),
        (1., 1.),
        (2., 1.),
        (1., f32::INFINITY),
        (f32::from_bits(1), 100.),
    ] {
        assert_eq!(Projection::new(1., 1., near, far), Err(Error::Depth));
    }
    assert_eq!(Extent::new(0, 1), Err(Error::Extent));
    let extent = Extent::new(10, 10).unwrap();
    assert_eq!(extent.sample_uv([10, 0], [0.; 2]), Err(Error::Sample));
    assert_eq!(extent.sample_uv([0, 0], [f32::NAN, 0.]), Err(Error::Sample));
    let camera = frame([0.; 3]).relative([0.; 3]).unwrap();
    assert!(camera.project([0., 0., 1.], extent).is_none());
}

#[test]
fn halton_period_matches_scale_and_handles_full_width_range() {
    for (input, output, count) in [
        (1920, 1920, 8),
        (960, 1920, 32),
        (1, u32::MAX, 256),
        (u32::MAX, u32::MAX, 8),
    ] {
        let sequence = Jitter::new(input, output).unwrap();
        assert_eq!(sequence.phases(), count);
        let mut samples = Vec::new();
        for index in 0..count {
            let sample = sequence.sample(index);
            assert!(sample.iter().all(|x| (-0.5..0.5).contains(x)));
            assert_eq!(sample, sequence.sample(index + count));
            assert!(!samples.contains(&sample));
            samples.push(sample);
        }
    }
    assert_eq!(Jitter::new(0, 1920), Err(Error::Extent));
}
