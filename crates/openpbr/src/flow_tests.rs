use super::*;

fn point(value: f32) -> Range {
    Range::point(value).unwrap()
}

#[test]
fn smooth_plastic_retains_diffuse_beside_delta_specular() {
    let key = Material::opaque([0.5; 3], 0.0, 1.5, 0.0, 0.0)
        .unwrap()
        .flow(false);
    assert_eq!(
        key.bits(),
        FlowKey::DIFFUSE | FlowKey::DIELECTRIC_REFLECTION | FlowKey::DELTA_REFLECTION
    );
    assert!(key.continuous());
    assert!(!key.pure_delta());
}

#[test]
fn fractional_metal_subsurface_and_emission_coexist() {
    let key = Material::opaque([0.5; 3], 0.5, 1.5, 0.4, 0.7)
        .unwrap()
        .flow(true);
    assert_eq!(
        key.bits(),
        FlowKey::DIFFUSE
            | FlowKey::DIELECTRIC_REFLECTION
            | FlowKey::CONDUCTOR_REFLECTION
            | FlowKey::SUBSURFACE
            | FlowKey::CONTINUOUS_REFLECTION
            | FlowKey::EMISSION
            | FlowKey::MIXED_METALNESS
    );
}

#[test]
fn filtered_binary_metalness_keeps_fractional_control_flow() {
    let mut scan = RangeScan::new();
    scan.extend(&[0.0, 1.0]).unwrap();
    let ranges = MaterialRanges::opaque(point(0.4), scan.finish().unwrap(), point(0.0)).unwrap();
    let key = ranges.flow(InterfaceIor::UNKNOWN, false);
    assert_eq!(
        key.bits() & (FlowKey::DIELECTRIC_REFLECTION | FlowKey::CONDUCTOR_REFLECTION),
        FlowKey::DIELECTRIC_REFLECTION | FlowKey::CONDUCTOR_REFLECTION
    );
}

#[test]
fn pure_subsurface_removes_lambert_path() {
    let key = Material::opaque([0.5; 3], 0.5, 1.5, 0.0, 1.0)
        .unwrap()
        .flow(false);
    assert_eq!(
        key.bits() & (FlowKey::DIFFUSE | FlowKey::SUBSURFACE),
        FlowKey::SUBSURFACE
    );
}

#[test]
fn conductor_removes_all_nonmetal_base_paths() {
    let key = Material::opaque([0.5; 3], 0.0, 1.5, 1.0, 1.0)
        .unwrap()
        .flow(false);
    assert_eq!(
        key.bits(),
        FlowKey::CONDUCTOR_REFLECTION | FlowKey::DELTA_REFLECTION
    );
    assert!(key.pure_delta());
}

#[test]
fn black_base_removes_diffuse_and_subsurface_without_removing_specular() {
    let key = Material::opaque([0.0; 3], 0.5, 1.5, 0.0, 0.7)
        .unwrap()
        .flow(false);
    assert_eq!(
        key.bits(),
        FlowKey::DIELECTRIC_REFLECTION | FlowKey::CONTINUOUS_REFLECTION
    );
}

#[test]
fn matched_opaque_ior_removes_only_dielectric_reflection() {
    let matched = InterfaceIor::between(point(1.5), point(1.5)).unwrap();
    let key = MaterialRanges::constant(Material::opaque([0.5; 3], 0.4, 1.5, 0.0, 0.2).unwrap())
        .flow(matched, false);
    assert_eq!(key.bits(), FlowKey::DIFFUSE | FlowKey::SUBSURFACE);
}

#[test]
fn black_matched_opaque_is_a_pure_absorber() {
    let key = MaterialRanges::constant(Material::opaque([0.0; 3], 0.4, 1.0, 0.0, 0.5).unwrap())
        .flow(
            InterfaceIor::between(point(1.0), point(1.0)).unwrap(),
            false,
        );
    assert_eq!(key.bits(), 0);
    assert!(!key.continuous());
    assert!(!key.pure_delta());
}

#[test]
fn black_matched_fractional_metal_keeps_the_absorbing_selection_gate() {
    let key = MaterialRanges::constant(Material::opaque([0.0; 3], 0.4, 1.0, 0.5, 0.0).unwrap())
        .flow(
            InterfaceIor::between(point(1.0), point(1.0)).unwrap(),
            false,
        );
    assert_eq!(
        key.bits(),
        FlowKey::CONDUCTOR_REFLECTION | FlowKey::CONTINUOUS_REFLECTION | FlowKey::MIXED_METALNESS
    );
}

#[test]
fn known_nonmatching_media_remove_solid_delta_domain() {
    let ranges = MaterialRanges::constant(Material::dielectric(0.2, None).unwrap());
    let interface = InterfaceIor::between(point(1.0), point(1.333)).unwrap();
    let key = ranges.flow(interface, false);
    assert_eq!(
        key.bits(),
        FlowKey::DIELECTRIC_REFLECTION
            | FlowKey::SOLID_TRANSMISSION
            | FlowKey::CONTINUOUS_REFLECTION
            | FlowKey::CONTINUOUS_TRANSMISSION
    );
    assert_ne!(
        ranges.flow(InterfaceIor::UNKNOWN, false).bits() & FlowKey::DELTA_TRANSMISSION,
        0
    );
}

#[test]
fn near_matched_solid_ior_retains_delta_transmission_with_gpu_division_error() {
    let key = MaterialRanges::constant(Material::dielectric(0.2, None).unwrap()).flow(
        InterfaceIor::between(point(1.0), point(f32::from_bits(0x3f800001))).unwrap(),
        false,
    );
    assert_eq!(
        key.bits(),
        FlowKey::SOLID_TRANSMISSION
            | FlowKey::DIELECTRIC_REFLECTION
            | FlowKey::CONTINUOUS_REFLECTION
            | FlowKey::DELTA_TRANSMISSION
            | FlowKey::CONTINUOUS_TRANSMISSION
    );
}

#[test]
fn matched_ior_keeps_only_delta_domain_even_on_rough_solid() {
    let key = MaterialRanges::constant(Material::dielectric(0.8, None).unwrap()).flow(
        InterfaceIor::between(point(1.5), point(1.5)).unwrap(),
        false,
    );
    assert_eq!(
        key.bits(),
        FlowKey::SOLID_TRANSMISSION | FlowKey::DELTA_TRANSMISSION
    );
}

#[test]
fn matched_thin_ior_removes_reflection_while_keeping_absorbing_transmission() {
    let key = MaterialRanges::constant(Material::dielectric(0.8, Some(0.01)).unwrap()).flow(
        InterfaceIor::between(point(1.5), point(1.5)).unwrap(),
        false,
    );
    assert_eq!(
        key.bits(),
        FlowKey::THIN_TRANSMISSION | FlowKey::DELTA_TRANSMISSION
    );
}

#[test]
fn thin_uses_scaled_transmission_roughness_for_the_whole_closure() {
    let key = MaterialRanges::constant(Material::dielectric(0.02, Some(0.01)).unwrap()).flow(
        InterfaceIor::between(point(1.0), point(1.001)).unwrap(),
        false,
    );
    assert!(key.pure_delta());
}

#[test]
fn thin_threshold_keeps_both_domains_across_reciprocal_rounding() {
    let key = MaterialRanges::constant(
        Material::dielectric(f32::from_bits(0x3c72a950), Some(0.01)).unwrap(),
    )
    .flow(
        InterfaceIor::between(point(1.41688), point(1.0)).unwrap(),
        false,
    );
    assert_eq!(
        key.bits(),
        FlowKey::THIN_TRANSMISSION
            | FlowKey::DIELECTRIC_REFLECTION
            | FlowKey::DELTA_REFLECTION
            | FlowKey::CONTINUOUS_REFLECTION
            | FlowKey::DELTA_TRANSMISSION
            | FlowKey::CONTINUOUS_TRANSMISSION
    );
}

#[test]
fn range_scan_rejects_a_whole_invalid_batch() {
    let mut scan = RangeScan::new();
    scan.extend(&[0.25, 0.5]).unwrap();
    assert_eq!(scan.extend(&[1.0, f32::NAN]), Err(FlowError::Range));
    assert_eq!(scan.finish().unwrap(), Range::new(0.25, 0.5).unwrap());
}

#[test]
fn plan_deduplicates_by_flow_in_binding_order() {
    let metal = Material::opaque([0.5; 3], 0.4, 1.5, 1.0, 0.0)
        .unwrap()
        .flow(false);
    let plastic = Material::opaque([0.2; 3], 0.4, 1.5, 0.0, 0.0)
        .unwrap()
        .flow(false);
    let plan = FlowPlan::new([metal, plastic, metal]).unwrap();
    assert_eq!(plan.keys().len(), 2);
    assert_eq!(plan.bindings()[0], plan.bindings()[2]);
    for (index, key) in [metal, plastic, metal].into_iter().enumerate() {
        assert_eq!(plan.keys()[plan.bindings()[index] as usize], key);
    }
}

#[test]
fn external_keys_reject_topologies_outside_the_current_subset() {
    let invalid = FlowKey::SOLID_TRANSMISSION
        | FlowKey::CONDUCTOR_REFLECTION
        | FlowKey::CONTINUOUS_REFLECTION
        | FlowKey::CONTINUOUS_TRANSMISSION;
    assert_eq!(FlowKey::from_bits(invalid), Err(FlowError::Key(invalid)));
}

#[test]
fn transmission_without_reflection_requires_the_matched_delta_domain() {
    let invalid = FlowKey::SOLID_TRANSMISSION | FlowKey::CONTINUOUS_TRANSMISSION;
    assert_eq!(FlowKey::from_bits(invalid), Err(FlowError::Key(invalid)));
}

#[test]
fn solid_shared_roughness_rejects_incoherent_reflection_and_transmission_domains() {
    let invalid = FlowKey::DIELECTRIC_REFLECTION
        | FlowKey::SOLID_TRANSMISSION
        | FlowKey::DELTA_REFLECTION
        | FlowKey::CONTINUOUS_TRANSMISSION;
    assert_eq!(FlowKey::from_bits(invalid), Err(FlowError::Key(invalid)));
}

#[test]
fn opaque_fractional_metal_requires_an_explicit_mixing_control() {
    let invalid = FlowKey::DIFFUSE
        | FlowKey::DIELECTRIC_REFLECTION
        | FlowKey::CONDUCTOR_REFLECTION
        | FlowKey::CONTINUOUS_REFLECTION;
    assert_eq!(FlowKey::from_bits(invalid), Err(FlowError::Key(invalid)));
}

#[test]
fn analysis_outputs_obey_external_key_contract() {
    let roughness = [point(0.0), point(0.3), Range::new(0.0, 1.0).unwrap()];
    for roughness in roughness {
        for metalness in [point(0.0), point(1.0), Range::new(0.0, 1.0).unwrap()] {
            for subsurface in [point(0.0), point(1.0), Range::new(0.0, 1.0).unwrap()] {
                let key = MaterialRanges::opaque(roughness, metalness, subsurface)
                    .unwrap()
                    .flow(InterfaceIor::UNKNOWN, true);
                assert_eq!(FlowKey::from_bits(key.bits()), Ok(key));
            }
        }
        for thin in [false, true] {
            let key = MaterialRanges::dielectric(roughness, thin)
                .unwrap()
                .flow(InterfaceIor::UNKNOWN, false);
            assert_eq!(FlowKey::from_bits(key.bits()), Ok(key));
        }
    }
}

#[test]
fn normal_distribution_bounds_cover_float32_shader_evaluation() {
    for roughness in [0., 0.009_999, 0.01, 0.010_001, 0.4, 1.] {
        for byte in 0..=255 {
            let distribution = byte as f32 / 255.;
            let range = point(roughness)
                .with_normal_distribution(point(distribution))
                .unwrap();
            let alpha = roughness * roughness;
            let spread = distribution * distribution;
            let value = (alpha * alpha + spread * spread).min(1.).sqrt().sqrt();
            assert!(range.min() <= value && value <= range.max());
        }
    }
}

#[test]
fn normal_distribution_underflow_retains_zero_lower_bound() {
    let range = point(1.0e-12).with_normal_distribution(point(0.)).unwrap();
    assert_eq!(range.min(), 0.);
}

#[test]
fn removing_emission_preserves_scattering_support() {
    let material = Material::opaque([0.5; 3], 0.4, 1.5, 0.5, 0.5).unwrap();
    assert_eq!(material.flow(true).without_emission(), material.flow(false));
}

#[test]
fn normal_distribution_rejects_out_of_domain_width() {
    assert!(
        point(0.4)
            .with_normal_distribution(Range::new(0., 1.1).unwrap())
            .is_err()
    );
}
