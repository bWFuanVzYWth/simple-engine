use crate::{MAX_COORD, NoiseError, PerlinNoise};

#[test]
fn empty_batch_succeeds() {
    PerlinNoise::new((7, 11))
        .sample_3d_batch(&[], &[], &[], &mut [])
        .unwrap();
}

#[test]
fn length_error_preserves_output() {
    let mut output = [-17.0; 2];
    assert_eq!(
        PerlinNoise::new((7, 11)).sample_3d_batch(&[0.0; 2], &[0.0], &[0.0; 2], &mut output),
        Err(NoiseError::LengthMismatch)
    );
    assert_eq!(output, [-17.0; 2]);
}

#[test]
fn invalid_tail_coordinate_preserves_entire_output() {
    let noise = PerlinNoise::new((7, 11));
    for invalid in [
        f32::NAN,
        f32::INFINITY,
        MAX_COORD.next_up(),
        (-MAX_COORD).next_down(),
    ] {
        let mut y = [0.0; 9];
        y[8] = invalid;
        let mut output = [-17.0; 9];
        assert_eq!(
            noise.sample_3d_batch(&[0.25; 9], &y, &[-0.5; 9], &mut output),
            Err(NoiseError::InvalidCoordinate { axis: 1, index: 8 })
        );
        assert_eq!(output, [-17.0; 9]);
    }
}

#[test]
fn tail_samples_preserve_x8_bits() {
    let noise = PerlinNoise::new((7, 11));
    let x: [f32; 9] = std::array::from_fn(|i| i as f32 * 0.125 - 0.5);
    let y: [f32; 9] = std::array::from_fn(|i| i as f32 * 0.375 + 0.25);
    let z: [f32; 9] = std::array::from_fn(|i| i as f32 * -0.25 - 1.0);
    for length in [1, 7, 8, 9] {
        let mut output = [0.0; 9];
        noise
            .sample_3d_batch(
                &x[..length],
                &y[..length],
                &z[..length],
                &mut output[..length],
            )
            .unwrap();
        for lane in 0..length {
            let expected = noise.noise_3d_x8([x[lane]; 8], [y[lane]; 8], [z[lane]; 8])[0];
            assert_eq!(output[lane].to_bits(), expected.to_bits(), "lane {lane}");
        }
    }
}

#[test]
fn closed_domain_endpoints_are_accepted() {
    let x = [-MAX_COORD, MAX_COORD];
    let mut output = [f32::NAN; 2];
    PerlinNoise::new((7, 11))
        .sample_3d_batch(&x, &[0.0; 2], &[0.0; 2], &mut output)
        .unwrap();
    assert_eq!(output.map(|v| v.to_bits() & 0x7fff_ffff), [0; 2]);
}
