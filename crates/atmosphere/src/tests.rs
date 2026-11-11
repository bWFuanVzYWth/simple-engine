use super::*;

#[test]
fn uniform_solar_disk_integrates_to_normal_irradiance() {
    let mut p = Model::new().unwrap().parameters;
    // 较大半角能区分投影立体角与未投影立体角，不只测试太阳的小角近似。
    p.sun[2] = 0.5;
    let radiance = p.sun_radiance_rgb();
    let irradiance = p.solar_irradiance_rgb();
    let steps = 4096;
    let step = f64::from(p.sun[2]) / f64::from(steps);
    let integral: f64 = (0..steps)
        .map(|i| {
            let angle = (f64::from(i) + 0.5) * step;
            2.0 * std::f64::consts::PI * angle.sin() * angle.cos() * step
        })
        .sum();
    for i in 0..3 {
        assert!(
            (f64::from(radiance[i]) * integral - f64::from(irradiance[i])).abs()
                < f64::from(irradiance[i].abs()) * 1e-6
        );
    }
}

#[test]
fn cancelled_record_rebuilds_the_same_view() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    let view = View::default();
    frame.prepare(view, &model.calibration).unwrap();
    frame.commit();
    frame.invalidate_view();
    assert_eq!(
        frame.prepare(view, &model.calibration).unwrap(),
        Update {
            sky: true,
            aerial: true
        }
    );
}

#[test]
fn physical_profile_has_ordered_heights_and_nonnegative_coefficients() {
    let model = Model::new().unwrap();
    let p = model.parameters;
    let profile = &model.data[p.offsets0[2] as usize..][..p.offsets1[2] as usize * 7];
    let heights: Vec<_> = profile
        .as_chunks::<7>()
        .0
        .iter()
        .map(|row| row[0][0])
        .collect();
    assert!(heights.windows(2).all(|h| h[0] < h[1]));
    for row in profile.as_chunks::<7>().0 {
        assert!(
            row[1..]
                .iter()
                .flatten()
                .all(|x| x.is_finite() && *x >= 0.0)
        );
    }
    assert!(p.solar_irradiance().iter().all(|x| *x > 0.0));
}

#[test]
fn unchanged_view_reuses_both_caches() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    let view = View::default();
    assert_eq!(
        frame.prepare(view, &model.calibration).unwrap(),
        Update {
            sky: true,
            aerial: true
        }
    );
    frame.commit();
    assert_eq!(
        frame.prepare(view, &model.calibration).unwrap(),
        Update::default()
    );
}

#[test]
fn azimuth_rotation_reuses_axisymmetric_sky() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    let view = View {
        sun_direction: [0.0, 0.6, 0.8],
        ..View::default()
    };
    frame.prepare(view, &model.calibration).unwrap();
    frame.commit();
    let mapping = frame.mapping;
    let changed = View {
        sun_direction: [0.8, 0.6, 0.0],
        ..view
    };
    assert_eq!(
        frame.prepare(changed, &model.calibration).unwrap(),
        Update {
            sky: false,
            aerial: true
        }
    );
    assert_eq!(frame.mapping, mapping);
}

#[test]
fn projection_changes_only_aerial() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    let view = View::default();
    frame.prepare(view, &model.calibration).unwrap();
    frame.commit();
    let changed = View {
        right: [2.0, 0.0, 0.0],
        ..view
    };
    assert_eq!(
        frame.prepare(changed, &model.calibration).unwrap(),
        Update {
            sky: false,
            aerial: true
        }
    );
}

#[test]
fn height_invalidates_both_observer_caches() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    frame.prepare(View::default(), &model.calibration).unwrap();
    frame.commit();
    assert_eq!(
        frame
            .prepare(
                View {
                    altitude_km: 12.0,
                    ..View::default()
                },
                &model.calibration
            )
            .unwrap(),
        Update {
            sky: true,
            aerial: true
        }
    );
}

#[test]
fn sun_elevation_invalidates_both_observer_caches() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    frame
        .prepare(
            View {
                sun_direction: [0.0, 0.6, 0.8],
                ..View::default()
            },
            &model.calibration,
        )
        .unwrap();
    frame.commit();
    assert_eq!(
        frame
            .prepare(
                View {
                    sun_direction: [0.0, 0.8, 0.6],
                    ..View::default()
                },
                &model.calibration
            )
            .unwrap(),
        Update {
            sky: true,
            aerial: true
        }
    );
}

#[test]
fn color_multiplier_changes_only_aerial() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    frame.prepare(View::default(), &model.calibration).unwrap();
    frame.commit();
    assert_eq!(
        frame
            .prepare(
                View {
                    radiance_scale: [0.5, 1.0, 2.0],
                    ..View::default()
                },
                &model.calibration
            )
            .unwrap(),
        Update {
            sky: false,
            aerial: true
        }
    );
}

#[test]
fn rejected_view_preserves_valid_cache_key() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    frame.prepare(View::default(), &model.calibration).unwrap();
    frame.commit();
    assert!(
        frame
            .prepare(
                View {
                    right: [0.0, 0.0, 1.0],
                    ..View::default()
                },
                &model.calibration
            )
            .is_err()
    );
    assert_eq!(
        frame.prepare(View::default(), &model.calibration).unwrap(),
        Update::default()
    );
}

#[test]
fn replacing_an_unrecorded_change_restores_mapping() {
    let model = Model::new().unwrap();
    let mut frame = FrameState::new(model.parameters);
    frame.prepare(View::default(), &model.calibration).unwrap();
    frame.commit();
    let mapping = frame.mapping;
    frame
        .prepare(
            View {
                altitude_km: 12.0,
                ..View::default()
            },
            &model.calibration,
        )
        .unwrap();
    assert_eq!(
        frame.prepare(View::default(), &model.calibration).unwrap(),
        Update::default()
    );
    assert_eq!(frame.mapping, mapping);
}

#[test]
fn chart_rows_cover_ordered_elevation_intervals() {
    let calibration = mapping::Calibration::new().unwrap();
    for height in [0.0, 0.2, 120.0, 400.0] {
        let (cache, layout) = calibration.cache(height, 0.1);
        assert!(cache.iter().flatten().all(|x| x.is_finite()));
        let counts = [layout[2], layout[3] - layout[2], SKY_SIZE - layout[3]];
        let mut row = 12;
        for (chart, count) in counts.into_iter().enumerate() {
            if count == 0 {
                continue;
            }
            let rows = &cache[row..row + count as usize];
            assert!(rows.windows(2).all(|v| v[0][1] <= v[1][1]));
            assert_eq!(rows[0][1], cache[chart * 4][2]);
            assert_eq!(rows.last().unwrap()[1], cache[chart * 4][3]);
            row += count as usize;
        }
    }
}

#[test]
fn aerial_basis_preserves_ndc_center_and_axis_scales() {
    let view = View {
        forward: [1.0, 0.0, 0.0],
        right: [0.0, 0.0, -2.0],
        up: [0.0, 0.5, 0.0],
        ..View::default()
    }
    .validated()
    .unwrap();
    let p = view.aerial_parameters();
    let direction =
        |x: f32, y: f32| std::array::from_fn::<_, 3, _>(|i| p[0][i] * x + p[1][i] * y + p[3][i]);
    assert_eq!(direction(0.0, 0.0), [1.0, 0.0, 0.0]);
    assert_eq!(direction(1.0, 1.0), [1.0, 0.5, -2.0]);
}
