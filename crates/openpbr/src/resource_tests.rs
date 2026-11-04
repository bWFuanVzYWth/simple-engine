use super::*;
use crate::FlowPlan;

fn tint(min: f32, max: f32) -> [Range; 3] {
    [Range::new(min, max).unwrap(); 3]
}

fn opaque(color: ColorRanges) -> FlowKey {
    MaterialBinding::new(
        MaterialRanges::constant(crate::Material::opaque([1.0; 3], 0.9, 1.5, 0.0, 0.0).unwrap()),
        color,
        InterfaceIor::UNKNOWN,
        false,
    )
    .unwrap()
    .flow()
}

#[test]
fn nearest_cutout_excludes_rejected_color_and_accepts_the_threshold() {
    let mut scan = ColorScan::new(Coverage::AlphaAtLeast(0.5), Filtering::Nearest).unwrap();
    scan.extend(Texels::LinearRgba(&[
        [1.0, 1.0, 1.0, 0.49],
        [0.0, 0.25, 0.0, 0.5],
    ]))
    .unwrap();
    let color = scan.finish(tint(1.0, 1.0)).unwrap().rgb();
    assert_eq!(color[0], Range::point(0.0).unwrap());
    assert_eq!(color[1], Range::point(0.25).unwrap());
    assert_eq!(color[2], Range::point(0.0).unwrap());
}

#[test]
fn filtered_cutout_keeps_transparent_neighbors_that_can_contribute_color() {
    let mut scan = ColorScan::new(Coverage::AlphaAtLeast(0.5), Filtering::Convex).unwrap();
    scan.extend(Texels::LinearRgba(&[
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]))
    .unwrap();
    let red = scan.finish(tint(1.0, 1.0)).unwrap().rgb()[0];
    // 两纹素各半混合恰好通过alpha，红色仍非零。
    assert!(red.min() <= 0.5 && red.max() >= 0.5);
}

#[test]
fn srgb8_coverage_uses_normalized_alpha_and_preserves_zero_channels() {
    let mut scan = ColorScan::new(Coverage::AlphaAtLeast(0.5), Filtering::Nearest).unwrap();
    scan.extend(Texels::Srgb8Rgba(&[255, 255, 255, 127, 0, 1, 0, 128]))
        .unwrap();
    let color = scan.finish(tint(1.0, 1.0)).unwrap().rgb();
    assert_eq!(color[0].max(), 0.0);
    assert_eq!(color[2].max(), 0.0);
    assert!(color[1].max() > 0.0);
}

#[test]
fn nonnegative_tint_range_covers_all_reachable_products() {
    let mut scan = ColorScan::new(Coverage::Opaque, Filtering::Nearest).unwrap();
    scan.extend(Texels::LinearRgba(&[[0.25, 0.5, 0.75, 1.0]]))
        .unwrap();
    let rgb = scan.finish(tint(0.0, 0.5)).unwrap().rgb();
    assert_eq!(rgb.map(Range::min), [0.0; 3]);
    assert_eq!(rgb.map(Range::max), [0.125, 0.25, 0.375]);
}

#[test]
fn black_tint_removes_diffuse_without_removing_dielectric_reflection() {
    let mut scan = ColorScan::new(Coverage::Opaque, Filtering::Nearest).unwrap();
    scan.extend(Texels::Srgb8Rgba(&[255, 128, 64, 255]))
        .unwrap();
    assert_eq!(
        opaque(scan.finish(tint(0.0, 0.0)).unwrap()).bits(),
        FlowKey::DIELECTRIC_REFLECTION | FlowKey::CONTINUOUS_REFLECTION
    );
}

#[test]
fn all_transparent_resource_has_a_valid_black_domain() {
    let mut scan = ColorScan::new(Coverage::AlphaAtLeast(0.5), Filtering::Nearest).unwrap();
    scan.extend(Texels::Srgb8Rgba(&[255, 255, 255, 0])).unwrap();
    assert_eq!(
        scan.finish(tint(0.0, 1.0)).unwrap().rgb().map(Range::max),
        [0.0; 3]
    );
}

#[test]
fn invalid_batch_preserves_previous_resource_range() {
    let mut scan = ColorScan::new(Coverage::Opaque, Filtering::Nearest).unwrap();
    scan.extend(Texels::LinearRgba(&[[0.0, 0.0, 0.0, 1.0]]))
        .unwrap();
    assert!(
        scan.extend(Texels::LinearRgba(&[[1.0; 4], [f32::NAN, 0.0, 0.0, 1.0]]))
            .is_err()
    );
    assert_eq!(
        scan.finish(tint(1.0, 1.0)).unwrap().rgb().map(Range::max),
        [0.0; 3]
    );
}

#[test]
fn actual_color_resources_deduplicate_equal_control_flow() {
    let black = opaque(ColorRanges::constant([0.0; 3]).unwrap());
    let red = opaque(ColorRanges::constant([1.0, 0.0, 0.0]).unwrap());
    let blue = opaque(ColorRanges::constant([0.0, 0.0, 1.0]).unwrap());
    let plan = FlowPlan::new([red, black, blue]).unwrap();
    assert_eq!(plan.keys().len(), 2);
    assert_eq!(plan.bindings()[0], plan.bindings()[2]);
    assert_ne!(plan.bindings()[0], plan.bindings()[1]);
}

#[test]
fn opaque_coverage_retains_color_even_when_texture_alpha_is_zero() {
    let mut scan = ColorScan::new(Coverage::Opaque, Filtering::Nearest).unwrap();
    scan.extend(Texels::LinearRgba(&[[0.25, 0.5, 0.75, 0.0]]))
        .unwrap();
    assert_eq!(
        scan.finish(tint(1.0, 1.0)).unwrap().rgb().map(Range::max),
        [0.25, 0.5, 0.75]
    );
}
