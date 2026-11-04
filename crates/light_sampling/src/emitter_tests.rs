use crate::{
    SELECTORS,
    emitter::{Coverage, EmissionProposal, EmitterAlias},
};

const SQUARE: [[f32; 3]; 4] = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
const UV: [[f32; 2]; 4] = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];

#[test]
fn sparse_texels_have_area_density_matching_the_actual_alias_mass() {
    let proposal = EmissionProposal::new([2, 2], &[1., 0., 0., 3.], &[255; 4])
        .unwrap()
        .unwrap();
    let alias = EmitterAlias::build(SQUARE, UV, Coverage::Opaque, &proposal)
        .unwrap()
        .unwrap();
    let words = alias.words();
    let grid = words[4] as usize;
    let entries = words[5] as usize;
    assert_eq!(&words[grid..grid + 4], &[0, u32::MAX, u32::MAX, 1]);
    for (cell, expected) in [1.0, 3.0].into_iter().enumerate() {
        let row = &words[entries + cell * 8..][..8];
        assert_eq!(f32::from_bits(row[5]), expected);
        assert_eq!(f32::from_bits(row[6]), 0.25);
        assert_eq!(
            f64::from(row[2]) / f64::from(SELECTORS),
            f64::from(expected) * 0.25
        );
    }
}

#[test]
fn triangle_texel_clipping_preserves_analytic_physical_areas_and_barycentric_support() {
    let vertices = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [0., 2., 0.]];
    let uv = [[0., 0.], [1., 0.], [0., 1.], [0., 1.]];
    let proposal = EmissionProposal::new([2, 2], &[1., 2., 4., 8.], &[255; 4])
        .unwrap()
        .unwrap();
    let alias = EmitterAlias::build(vertices, uv, Coverage::Opaque, &proposal)
        .unwrap()
        .unwrap();
    let words = alias.words();
    let entries = words[5] as usize;
    let patches = words[6] as usize;
    let grid = words[4] as usize;
    assert_eq!(&words[grid..grid + 4], &[0, 1, 2, u32::MAX]);
    for (cell, expected_area) in [1.0, 0.5, 0.5].into_iter().enumerate() {
        let row = &words[entries + cell * 8..][..8];
        assert_eq!(f32::from_bits(row[6]), expected_area);
        let mut area_sum = 0.0;
        for patch in row[3]..row[3] + row[4] {
            let data = &words[patches + patch as usize * 8..][..8];
            assert_eq!(data[0], 0);
            area_sum += f32::from_bits(data[1]);
            for bary in data[2..].as_chunks::<2>().0 {
                let b = [f32::from_bits(bary[0]), f32::from_bits(bary[1])];
                assert!(b[0] >= 0. && b[1] >= 0. && b[0] + b[1] <= 1.0 + 1e-6);
                let lo = [(cell % 2) as f32 * 0.5, (cell / 2) as f32 * 0.5];
                assert!((0..2).all(|a| b[a] >= lo[a] - 1e-6 && b[a] <= lo[a] + 0.5 + 1e-6));
            }
        }
        assert_eq!(area_sum, expected_area);
        assert!(
            (f32::from_bits(row[5]) * expected_area - row[2] as f32 / SELECTORS as f32).abs()
                < 1e-7
        );
    }
}

#[test]
fn cutout_coverage_uses_the_declared_threshold_and_opaque_ignores_alpha() {
    let proposal = EmissionProposal::new([2, 2], &[1.; 4], &[255, 0, 127, 128])
        .unwrap()
        .unwrap();
    let alias = EmitterAlias::build(SQUARE, UV, Coverage::AlphaThreshold(128), &proposal)
        .unwrap()
        .unwrap();
    let words = alias.words();
    let grid = words[4] as usize;
    assert_eq!(&words[grid..grid + 4], &[0, u32::MAX, u32::MAX, 1]);
    assert!(
        EmitterAlias::build(SQUARE, UV, Coverage::Opaque, &proposal)
            .unwrap()
            .is_none()
    );
}

#[test]
fn unsupported_mapping_and_degenerate_sources_keep_the_uniform_fallback() {
    let proposal = EmissionProposal::new([2, 2], &[1., 0., 0., 3.], &[255; 4])
        .unwrap()
        .unwrap();
    let mut nonplanar = SQUARE;
    nonplanar[3][2] = 1.;
    assert!(
        EmitterAlias::build(nonplanar, UV, Coverage::Opaque, &proposal)
            .unwrap()
            .is_none()
    );
    let mut outside = UV;
    outside[0][0] = -0.1;
    assert!(
        EmitterAlias::build(SQUARE, outside, Coverage::Opaque, &proposal)
            .unwrap()
            .is_none()
    );
    assert!(
        EmitterAlias::build([[0.; 3]; 4], UV, Coverage::Opaque, &proposal)
            .unwrap()
            .is_none()
    );
    assert!(
        EmissionProposal::new([2, 2], &[1.; 4], &[255; 4])
            .unwrap()
            .is_none()
    );
    assert!(
        EmissionProposal::new([2, 2], &[0.; 4], &[255; 4])
            .unwrap()
            .is_none()
    );
}

#[test]
fn invalid_proposal_domains_are_rejected_before_constructing_gpu_words() {
    assert!(EmissionProposal::new([2, 2], &[1.; 3], &[255; 4]).is_err());
    assert!(EmissionProposal::new([1, 1], &[f32::NAN], &[255]).is_err());
    assert!(EmissionProposal::new([1, 1], &[-1.], &[255]).is_err());
}

#[test]
fn table_free_registry_sampling_matches_its_inverse_finite_grid_probabilities() {
    for count in 1..=257 {
        let mut start = 0;
        for index in 0..count {
            let pmf = crate::uniform_pmf(count, index).unwrap();
            let mass = (pmf * SELECTORS as f32) as u32;
            assert!(mass > 0);
            for selector in [start, start + mass - 1] {
                let selected = crate::uniform_sample(count, selector).unwrap().unwrap();
                assert_eq!(selected.index, index);
                assert_eq!(selected.pmf, pmf);
            }
            start += mass;
        }
        assert_eq!(start, SELECTORS);
    }
    assert!(crate::uniform_sample(0, 0).unwrap().is_none());
    assert!(crate::uniform_sample(SELECTORS + 1, 0).is_err());
    assert!(crate::uniform_pmf(SELECTORS + 1, 0).is_err());
}
