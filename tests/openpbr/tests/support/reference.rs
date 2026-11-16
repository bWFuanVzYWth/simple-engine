//! 独立f64解析/积分模型；不调用生产Slang或复刻GPU夹具调度。
use super::fixture::Case;

pub(super) fn reference_beta_integral(a: f64, b: f64) -> f64 {
    let center = (a / b).ln();
    let steps = 16384_u32;
    let width = 80. / f64::from(steps);
    let mut sum = 0.;
    for index in 0..=steps {
        let v = center - 40. + f64::from(index) * width;
        let value = (-a * (-v).exp().ln_1p() - b * v.exp().ln_1p()).exp();
        let weight = if index == 0 || index == steps {
            1.
        } else if index % 2 == 0 {
            2.
        } else {
            4.
        };
        sum += weight * value;
    }
    sum * width / 3.
}

pub(super) fn reference_null_metal_sample(case: &Case) -> Option<(f64, f64)> {
    let words = case.material.to_words();
    let weight = f64::from(f32::from_bits(words[5]));
    if f64::from(case.u[2]) >= weight {
        return None;
    }
    let roughness = f64::from(f32::from_bits(words[3]));
    let alpha = roughness * roughness;
    let wi = [
        f64::from(case.view[0]),
        f64::from(case.view[1]),
        f64::from(case.view[2]).abs(),
    ];
    let length = (alpha * alpha * (wi[0] * wi[0] + wi[1] * wi[1]) + wi[2] * wi[2]).sqrt();
    let visible = [
        alpha * wi[0] / length,
        alpha * wi[1] / length,
        wi[2] / length,
    ];
    let s = 1. + wi[0].hypot(wi[1]);
    let k = (1. - alpha * alpha) * s * s / (s * s + alpha * alpha * wi[2] * wi[2]);
    let z = 1. - (1. + k * visible[2]) * f64::from(case.u[1]);
    let phi = std::f64::consts::TAU * f64::from(case.u[0]);
    let sine = (1. - z * z).max(0.).sqrt();
    let half = [
        alpha * (sine * phi.cos() + visible[0]),
        alpha * (sine * phi.sin() + visible[1]),
        (z + visible[2]).max(0.),
    ];
    let half_length = half.iter().map(|x| x * x).sum::<f64>().sqrt();
    let half = half.map(|x| x / half_length);
    let cosine = wi.iter().zip(half).map(|(a, b)| a * b).sum::<f64>();
    let outgoing = std::array::from_fn::<_, 3, _>(|axis| -wi[axis] + 2. * cosine * half[axis]);
    if outgoing[2] <= 0. {
        return None;
    }
    let denominator = half[0] * half[0] + half[1] * half[1] + alpha * alpha * half[2] * half[2];
    let distribution = alpha * alpha / (std::f64::consts::PI * denominator * denominator);
    let outgoing_length = (alpha * alpha * (outgoing[0] * outgoing[0] + outgoing[1] * outgoing[1])
        + outgoing[2] * outgoing[2])
        .sqrt();
    let factor = 0.5 * outgoing[2] / (wi[2] * outgoing_length + outgoing[2] * length);
    Some((
        weight * distribution * factor * (1. - cosine).powi(5),
        weight * distribution / (2. * (k * wi[2] + length)),
    ))
}
