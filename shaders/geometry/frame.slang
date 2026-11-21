/// 单位方向的切向基；仅提供几何方向，不改变采样随机维度或方位约定。
module frame;

namespace geometry {
/// normal须为有限单位向量；朝向与cross(z或y, normal)归一化相同。
public func unit_tangent(normal: float3)->float3 {
    let use_z = abs(normal.z) < 0.999;
    let other = use_z ? normal.y : normal.z;
    let tangent = use_z ? float3(-normal.y, normal.x, 0) : float3(normal.z, 0, -normal.x);
    return tangent * rsqrt(normal.x * normal.x + other * other);
}

/// 几何外法线、贴图外法线与指向观察者的方向均为单位向量。保留有效斜率；
/// 理想反射落入几何面时，在原切向方位上取满足反射半球的最大倾角。
public func mapped_normal_for_view(geometric: float3, mapped: float3, view: float3)->float3 {
    let facing = dot(geometric, view) < 0 ? -1.0 : 1.0;
    let n = geometric * facing;
    let s = dot(mapped, geometric) < 0 ? -mapped * facing : mapped * facing;
    let cosine = dot(n, view);
    let minimum = min(0.9 * cosine, 0.01);
    let sn = dot(s, n);
    if (2.0 * dot(s, view) * sn - cosine >= minimum && dot(s, view) > 0)
        return s * facing;
    let slope = s - sn * n;
    let squared = dot(slope, slope);
    if (squared <= 1e-12)
        return geometric;
    let tangent = slope * rsqrt(squared);
    let x = dot(tangent, view);
    // 把视线投影到斜率平面。反射方向的法向分量固定为 minimum，
    // 切向分量取正根，再用半角向量恢复法线。负 x 时有理化以保留掠角精度。
    let reflected_x = sqrt(x * x + (cosine - minimum) * (cosine + minimum));
    if (x == 0 && cosine == 0)
        return geometric;
    let half_vector = x < 0 ? (cosine - minimum) * tangent + (reflected_x - x) * n
                            : (reflected_x + x) * tangent + (cosine + minimum) * n;
    return normalize(half_vector) * facing;
}
}
