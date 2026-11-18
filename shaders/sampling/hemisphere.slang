/// 相对于单位法线的半球采样。
module hemisphere;
import math.constants;
import geometry.frame;

namespace sampling {
/// normal 须为单位向量，u 属于 [0, 1)²；返回同坐标系内 normal 正半球的单位方向。
/// PDF 为 cos(theta)/math::pi，采用立体角量度 sr^-1，不包含随机数生成。
/// 极坐标圆盘映射：https://pbr-book.org/4ed/Sampling_Algorithms/Sampling_Multidimensional_Functions
public func cosine_hemisphere(normal: float3, u: float2)->float3 {
    let phi = math::tau * u.y;
    let tangent = geometry::unit_tangent(normal);
    let bitangent = cross(normal, tangent);
    var sine : float;
    var cosine : float;
    sincos(phi, sine, cosine);
    return sqrt(u.x) * (cosine * tangent + sine * bitangent) + sqrt(1.0 - u.x) * normal;
}
}
