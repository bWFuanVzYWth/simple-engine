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
}
