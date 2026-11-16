/// 无资源绑定的单面 Lambertian；方向约定均为离开表面，不执行路径或光源策略。
/// 来源：https://pbr-book.org/4ed/Reflection_Models/Diffuse_Reflection
module bsdf;
import math.constants;

import pt.sampling;

namespace pt
{
/// 同一坐标系中的采样方向与立体角 PDF。
public struct DirectionSample
{
    /// 离开表面的单位方向，与采样时的 normal 同坐标系。
    public var direction : float3;
    /// 立体角概率密度，单位 sr^-1。
    public var pdf : float;
};

/// albedo 属于 [0, 1]³；返回 BRDF f，不含余弦，单位 sr^-1。
/// 调用方须确认入射与出射单位方向均指离开表面，且处于单位法线正半球；本函数不检查方向。
public func lambertian_eval(albedo: float3)->float3
{
    return albedo * math::frac_1_pi;
}

/// normal、wi 须为单位向量，wi 指离开表面；返回立体角 PDF，负半球返回零。
/// 采用此单面 BSDF 的另一出射方向也须位于 normal 正半球，由调用方约束。
public func lambertian_pdf(normal: float3, wi: float3)->float
{
    return saturate(dot(normal, wi)) * math::frac_1_pi;
}

/// normal 须为单位向量，u 属于 [0, 1)²；采样其正半球并返回立体角 PDF。
/// 此分布下 f*cos(theta)/pdf 解析约消为 albedo，另一出射方向须位于同一正半球。
public func lambertian_sample(normal: float3, u: float2)->DirectionSample
{
    var sample : DirectionSample;
    sample.direction = cosine_hemisphere(normal, u);
    sample.pdf = lambertian_pdf(normal, sample.direction);
    return sample;
}
}
