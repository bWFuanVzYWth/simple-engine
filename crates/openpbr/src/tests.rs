use super::*;
use sha2::{Digest, Sha256};

#[test]
fn author_energy_bytes_match_the_locked_source() {
    assert_eq!(
        energy_bytes().len(),
        ENERGY_EXTENT.iter().map(|n| *n as usize).product::<usize>() * 8
    );
    let expected: [u8; 32] = std::array::from_fn(|index| {
        u8::from_str_radix(&ENERGY_SHA256[2 * index..2 * index + 2], 16)
            .expect("锁定摘要为32字节十六进制")
    });
    let actual: [u8; 32] = Sha256::digest(energy_bytes()).into();
    assert_eq!(actual, expected);
}

#[test]
fn energy_coefficients_are_finite_binary16_values() {
    for bytes in energy_bytes().as_chunks::<2>().0 {
        assert_ne!(u16::from_le_bytes([bytes[0], bytes[1]]) & 0x7c00, 0x7c00);
    }
}

#[test]
fn continuous_material_controls_reach_the_gpu_without_byte_quantization() {
    let material = Material::opaque([0.2, 0.3, 0.4], 0.37, 1.5, 0.12345, 0.6789).unwrap();
    let words = material.to_words();
    assert_eq!(f32::from_bits(words[5]), 0.12345);
    assert_eq!(f32::from_bits(words[6]), 0.6789);
}

#[test]
fn dielectric_topology_keeps_explicit_thickness() {
    let solid = Material::dielectric(0.1, None).unwrap().to_words();
    let thin = Material::dielectric(0.1, Some(0.002)).unwrap().to_words();
    assert_eq!(solid[8], 1);
    assert_eq!(thin[8], 3);
    assert_eq!(f32::from_bits(thin[7]), 0.002);
}

#[test]
fn invalid_material_fields_report_their_source_field() {
    assert_eq!(
        Material::opaque([f32::NAN, 0.2, 0.2], 0.2, 1.5, 0.0, 0.0),
        Err(Error::BaseColor)
    );
    assert_eq!(
        Material::opaque([0.2; 3], -0.1, 1.5, 0.0, 0.0),
        Err(Error::Roughness)
    );
    assert_eq!(
        Material::opaque([0.2; 3], 0.2, 0.0, 0.0, 0.0),
        Err(Error::Ior)
    );
    assert_eq!(
        Material::opaque([0.2; 3], 0.2, 1.5, 1.1, 0.0),
        Err(Error::Metalness)
    );
    assert_eq!(
        Material::opaque([0.2; 3], 0.2, 1.5, 0.0, f32::INFINITY),
        Err(Error::Subsurface)
    );
    assert_eq!(Material::dielectric(0.2, Some(0.0)), Err(Error::Thickness));
}

#[test]
fn medium_preserves_ior_and_absorption_without_color_conversion() {
    assert_eq!(
        Medium::new(1.333, [0.1, 0.2, 0.3]).unwrap().to_array(),
        [1.333, 0.1, 0.2, 0.3]
    );
    assert_eq!(Medium::new(1.5, [0.0, -0.1, 0.0]), Err(Error::Extinction));
    assert_eq!(Medium::new(f32::NAN, [0.0; 3]), Err(Error::Ior));
}
