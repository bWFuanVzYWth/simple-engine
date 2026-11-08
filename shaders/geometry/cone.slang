/// 单位轴的均匀立体角圆锥；半角属于 (0,pi]，方向与轴须为单位向量。
module cone;

namespace geometry
{
/// 球面弦长平方；小角度下仍保留有效位，不计算 1-cos(angle)。
public float cone_chord_squared(float angle)
{
    float halfAngle = 0.5 * angle;
    // GPU sin 的绝对误差保证不足以约束微小圆盘的相对误差；此区间的余项低于 f32 舍入。
    float squared = halfAngle * halfAngle;
    float halfSine = halfAngle < 0.1 ? halfAngle * (1 - squared * (1.0 / 6.0 - squared / 120.0))
                                     : sin(halfAngle);
    return 4 * halfSine * halfSine;
}

public float uniform_cone_pdf(float angle)
{
    return 1 / (3.141592653589793 * cone_chord_squared(angle));
}

/// 发光域和反向 PDF 共用此谓词；避免单位向量点积的末位误差放大小角度边界。
public bool cone_contains(float3 axis, float angle, float3 direction)
{
    float3 chord = direction - axis;
    return dot(chord, chord) <= cone_chord_squared(angle);
}

public float3 sample_uniform_cone(float3 axis, float angle, float2 u)
{
    float offset = 0.5 * cone_chord_squared(angle) * u.x;
    float z = 1 - offset;
    float radius = sqrt(offset * (2 - offset));
    float phi = 6.283185307179586 * u.y;
    float3 tangent =
        normalize(cross(abs(axis.z) < 0.999 ? float3(0, 0, 1) : float3(0, 1, 0), axis));
    return axis * z + radius * (cos(phi) * tangent + sin(phi) * cross(axis, tangent));
}
}
