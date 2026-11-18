/// 单位轴的均匀立体角圆锥；半角属于 (0,pi]，方向与轴须为单位向量。
module cone;
import math.constants;
import geometry.frame;

namespace sampling {
/// 球面弦长平方；小角度下仍保留有效位，不计算 1-cos(angle)。
public func cone_chord_squared(angle: float)->float {
    let half_angle = 0.5 * angle;
    // GPU sin 的绝对误差保证不足以约束微小圆盘的相对误差；此区间的余项低于 f32 舍入。
    let squared = half_angle * half_angle;
    let half_sine = half_angle < 0.1
                        ? half_angle * (1 + squared * (-1.0 / 6.0 + squared * (1.0 / 120.0)))
                        : sin(half_angle);
    return 4 * half_sine * half_sine;
}

public func uniform_cone_pdf(angle: float)->float {
    return uniform_cone_pdf_chord(cone_chord_squared(angle));
}

/// chord_squared为同一合法圆锥的弦长平方，可在CPU或采样所有者处预计算并共享。
public func uniform_cone_pdf_chord(chord_squared: float)->float {
    return math::frac_1_pi / chord_squared;
}

/// 发光域和反向 PDF 共用此谓词；避免单位向量点积的末位误差放大小角度边界。
public func cone_contains(axis: float3, angle: float, direction: float3)->bool {
    return cone_contains_chord(axis, cone_chord_squared(angle), direction);
}

/// 与采样和PDF共享弦长；保留同一单位方向支持谓词，不重新求角度函数。
public func cone_contains_chord(axis: float3, chord_squared: float, direction: float3)->bool {
    let chord = direction - axis;
    return dot(chord, chord) <= chord_squared;
}

/// axis须为单位向量，半角属于(0,pi]，u属于[0,1)²。
public func sample_uniform_cone(axis: float3, angle: float, u: float2)->float3 {
    return sample_uniform_cone_chord(axis, cone_chord_squared(angle), u);
}

/// axis须为单位向量，u属于[0,1)²；chord_squared须对应同一圆锥，属于(0,4]。
public func sample_uniform_cone_chord(axis: float3, chord_squared: float, u: float2)->float3 {
    let offset = 0.5 * chord_squared * u.x;
    let z = 1 - offset;
    let radius : float = sqrt(offset * (2 - offset));
    let phi = math::tau * u.y;
    let tangent = geometry::unit_tangent(axis);
    var sine : float;
    var cosine : float;
    sincos(phi, sine, cosine);
    return axis * z + radius * (cosine * tangent + sine * cross(axis, tangent));
}
}
