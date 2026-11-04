use crate::{AliasTable, Error, QuadArea, SELECTORS, pack_vertices};

fn check_table(weights: &[f64]) {
    let table = AliasTable::build(weights).unwrap();
    if weights.iter().all(|&w| w == 0.0) {
        assert!(table.is_empty());
        assert!(table.entries().is_empty());
        assert!(table.sample(0).unwrap().is_none());
        return;
    }
    assert_eq!(table.count() as usize, weights.len());
    let capacity = 1 << table.column_shift();
    let mut masses = vec![0u32; weights.len()];
    // 独立按每列的两个整数区间累计真实支持，不枚举2^24随机数或重建构造步骤。
    for (column, entry) in table.entries().iter().enumerate() {
        assert!(entry[0] <= capacity);
        assert_eq!(entry[3], 0);
        if entry[0] != 0 {
            assert!(column < weights.len());
            masses[column] += entry[0];
            for local in [0, entry[0] - 1] {
                let chosen = table
                    .sample((column as u32) * capacity + local)
                    .unwrap()
                    .unwrap();
                assert_eq!(chosen.index as usize, column);
            }
        }
        if entry[0] != capacity {
            assert!((entry[1] as usize) < weights.len());
            masses[entry[1] as usize] += capacity - entry[0];
            for local in [entry[0], capacity - 1] {
                let chosen = table
                    .sample((column as u32) * capacity + local)
                    .unwrap()
                    .unwrap();
                assert_eq!(chosen.index, entry[1]);
            }
        }
    }
    assert_eq!(
        masses.iter().map(|&v| u64::from(v)).sum::<u64>(),
        u64::from(SELECTORS)
    );
    for (index, (&mass, &weight)) in masses.iter().zip(weights).enumerate() {
        assert_eq!(mass != 0, weight > 0.0);
        assert_eq!(mass, table.entries()[index][2]);
        assert_eq!(table.pmf(index as u32), mass as f32 / SELECTORS as f32);
    }
    assert_eq!(table.pmf(weights.len() as u32), 0.0);
}

#[test]
fn alias_encoded_intervals_match_actual_pmf_for_small_weight_domains() {
    for len in 0..=6 {
        for code in 0usize..4usize.pow(len) {
            let mut value = code;
            let weights: Vec<_> = (0..len)
                .map(|_| {
                    let w = (value % 4) as f64;
                    value /= 4;
                    w
                })
                .collect();
            check_table(&weights);
        }
    }
}

#[test]
fn alias_preserves_extreme_positive_support_without_overflowing_the_sum() {
    check_table(&[
        f64::MAX,
        f64::MIN_POSITIVE,
        f64::MAX,
        f64::from_bits(1),
        0.0,
    ]);
    check_table(&[1.0]);
    check_table(&[0.0, 5.0, 0.0]);
}

#[test]
fn alias_rejects_invalid_weights_and_out_of_domain_selectors() {
    for value in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            AliasTable::build(&[1.0, value]),
            Err(Error::Weight(1))
        ));
    }
    let table = AliasTable::build(&[1.0]).unwrap();
    assert!(matches!(table.sample(SELECTORS), Err(Error::Selector)));
    assert!(matches!(
        AliasTable::default().sample(u32::MAX),
        Err(Error::Selector)
    ));
}

#[test]
fn quad_uses_both_real_triangles_with_matching_quantized_density() {
    let vertices = [
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 1.0, 0.0],
        [0.0, 1.0, 2.0],
    ];
    let quad = QuadArea::new(&vertices).unwrap();
    let areas = [1.0, 6.0_f64.sqrt()];
    assert!((quad.area() - areas.iter().sum::<f64>()).abs() < 1e-12);
    for (i, &density) in quad.densities().iter().enumerate() {
        let mass = [quad.split(), SELECTORS - quad.split()][i];
        assert!(
            (f64::from(density) * areas[i] - f64::from(mass) / f64::from(SELECTORS)).abs() < 1e-7
        );
    }
    assert_ne!(quad.split(), SELECTORS / 2);
    assert_eq!(quad.words()[3], 0);
    let [p, q, r] = pack_vertices(vertices);
    assert_eq!([p[0], p[1], p[2]], vertices[0]);
    assert_eq!([p[3], q[0], q[1]], vertices[1]);
    assert_eq!([q[2], q[3], r[0]], vertices[2]);
    assert_eq!([r[1], r[2], r[3]], vertices[3]);
}

#[test]
fn quad_allows_either_degenerate_half_without_giving_it_support() {
    let first = QuadArea::new(&[[0.; 3], [1., 0., 0.], [0., 1., 0.], [0., 1., 0.]]).unwrap();
    assert_eq!(first.split(), SELECTORS);
    assert_eq!(first.densities(), [2.0, 0.0]);
    let second = QuadArea::new(&[[0.; 3], [0.; 3], [1., 0., 0.], [0., 1., 0.]]).unwrap();
    assert_eq!(second.split(), 0);
    assert_eq!(second.densities(), [0.0, 2.0]);
}

#[test]
fn quad_rejects_no_area_and_invalid_shader_normal_domains() {
    assert!(matches!(QuadArea::new(&[[0.; 3]; 4]), Err(Error::Quad)));
    assert!(matches!(
        QuadArea::new(&[[f32::NAN; 3]; 4]),
        Err(Error::Quad)
    ));
    assert!(matches!(
        QuadArea::new(&[[0.; 3], [1e20, 0., 0.], [0., 1e20, 0.], [0.; 3]]),
        Err(Error::Quad)
    ));
    assert!(matches!(
        QuadArea::new(&[[0.; 3], [1e-30, 0., 0.], [0., 1e-30, 0.], [0.; 3]]),
        Err(Error::Quad)
    ));
}
