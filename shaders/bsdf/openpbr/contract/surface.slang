// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 公开表面边界：紧凑材料、能量资源与世界坐标结果；布局与Rust消费者共同维护。
module surface;
__exported import bsdf.openpbr.contract.energy_lut;
namespace openpbr
{
/// 48B 材料，CPU 内容见 Rust Material::to_words；命中贴图可替换有效 baseRGB。
public struct Material
{
    /// RGB 与线性 roughness，均属于 [0,1]。
    public var base_roughness : float4;
    /// opaque IOR、metalness、薄层 SSS weight、thin dielectric thickness。
    public var surface : float4;
    /// bit0=dielectric，bit1=thin dielectric；其余内容为零。
    public var control : uint4;
};

/// 原作者能量表的真实 Rgba16Float 3D 纹理及 normalized linear clamp LOD0 sampler。
/// 纹理为 44x32x159，不是颜色；资源所有权与绑定由调用方维持。
public struct Energy : IEnergyLut
{
    public var texture : Texture3D<float4>;
    public var sampler : SamplerState;
    public func lookup(uvw: float3)->float4
    {
        return texture.SampleLevel(sampler, uvw, 0.0);
    }
};

/// 反射事件；与 TRANSMISSION 互斥。
public static const let REFLECTION = 1u;
/// 透射事件；solid dielectric 更新 medium，thin 和薄层 SSS 不更新。
public static const let TRANSMISSION = 2u;
/// 漫反射／薄层 SSS 分量。
public static const let DIFFUSE = 4u;
/// 有限宽度微表面分量。
public static const let GLOSSY = 8u;
/// 离散 delta 事件，pdf 是概率质量；不能与立体角密度直接用于 MIS。
public static const let DISCRETE = 16u;

/// 完整闭包的连续方向响应与边缘 proposal PDF；response 已包含 |cos|，不是裸 BRDF。
public struct Evaluation
{
    public var response : float3;
    public var pdf : float;
};

/// 拒绝 proposal 时 events=0，无可消费的散射；其余内容规范为零／incident medium。
/// 有效 proposal 以 beta*=response/pdf 更新；不要再次乘 cos 或额外重复乘 eta²。
public struct Sample
{
    public var direction : float3;
    public var response : float3;
    public var pdf : float;
    /// eta_t/eta_i，供 eta-aware RR；响应已包含参考实现的 radiance eta 补偿。
    public var relative_eta : float;
    public var events : uint;
    /// 后续段的 float4(IOR,RGB extinction)，thin dielectric 透射仍保留入射介质。
    public var medium : float4;
};
}
