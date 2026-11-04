/// 无资源绑定的单面 Lambertian；方向约定均为离开表面，不执行路径或光源策略。
/// 来源：https://pbr-book.org/4ed/Reflection_Models/Diffuse_Reflection
module bsdf;

import pt.sampling;

namespace pt
{
/// 同一坐标系中的采样方向与立体角 PDF。
public struct DirectionSample
{
    /// 离开表面的单位方向，与采样时的 normal 同坐标系。
    public float3 direction;
    /// 立体角概率密度，单位 sr^-1。
    public float pdf;
};

/// albedo 属于 [0, 1]³；返回 BRDF f，不含余弦，单位 sr^-1。
/// 调用方须确认入射与出射单位方向均指离开表面，且处于单位法线正半球；本函数不检查方向。
public float3 lambertian_eval(float3 albedo)
{
    return albedo / pi;
}

/// normal、wi 须为单位向量，wi 指离开表面；返回立体角 PDF，负半球返回零。
/// 采用此单面 BSDF 的另一出射方向也须位于 normal 正半球，由调用方约束。
public float lambertian_pdf(float3 normal, float3 wi)
{
    return max(0.0, dot(normal, wi)) / pi;
}

/// normal 须为单位向量，u 属于 [0, 1)²；采样其正半球并返回立体角 PDF。
/// 此分布下 f*cos(theta)/pdf 解析约消为 albedo，另一出射方向须位于同一正半球。
public DirectionSample lambertian_sample(float3 normal, float2 u)
{
    DirectionSample sample;
    sample.direction = cosine_hemisphere(normal, u);
    sample.pdf = lambertian_pdf(normal, sample.direction);
    return sample;
}
}
