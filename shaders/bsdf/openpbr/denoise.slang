/// OpenPBR 方向能量降噪辅助量；不包含照明、吞吐、发光或曝光。
module denoise;

import bsdf.openpbr.api;
import bsdf.openpbr.flow;
import bsdf.openpbr.guide_composition;
import bsdf.openpbr.selection;
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/pt_subsurface.slang";
import "bsdf/openpbr/service/support/sanitization.slang";

namespace openpbr
{
/// 观察方向上的分量能量，线性工作空间 RGB；有限且裁剪到 [0,1]。
public struct DenoiseAlbedos
{
    public var diffuse : float3;
    public var specular : float3;
};

/// 与生产闭包一致的方向能量，不是 baseColor 或 F0 的别名。
/// dielectric 的 diffuse 表示透射能量代理；不包含介质 Beer 或界面滤色。
/// 输入范围、能量表和方向约定同 api；normal 为单位表面法线，view 指向观察者。
func denoise_albedos_impl<let Flow : uint, let Specialized : bool, Energy>(
    energy: Energy, material: Material, normal: float3, view: float3, incident: float4,
    transmitted: float4)
    ->DenoiseAlbedos
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    let dielectric : bool = Specialized
                                ? (Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) != 0
                                : (material.control.x & 1u) != 0;
    let thin : bool = Specialized ? (Flow & FLOW_THIN_TRANSMISSION) != 0
                                  : (material.control.x & 2u) != 0;
    let exiting : bool = dielectric && !thin && incident.x > transmitted.x;
    let cosine : float = abs(dot(normal, view)) * (exiting ? -1.0 : 1.0);
    var microfacet : PrimeFullOpenPbrMicrofacet;
    microfacet.alpha = float2(material.base_roughness.w * material.base_roughness.w);
    var result : DenoiseAlbedos = {};
    if (dielectric)
    {
        let ior : float = (exiting ? incident.x : transmitted.x) /
                          (exiting ? transmitted.x : incident.x);
        let reflected : float = primeFullOpenPbrMicrofacetDielectricMsCompensation(
                                    energy, microfacet, thin ? abs(cosine) : cosine, ior)
                                    .z;
        result.diffuse = float3(1.0 - reflected);
        result.specular = float3(reflected);
    }
    else
    {
        let metalness : float = Specialized && (Flow & FLOW_METAL_MIX) == 0
                                    ? ((Flow & FLOW_CONDUCTOR_REFLECTION) != 0 ? 1.0 : 0.0)
                                    : material.surface.y;
        if ((!Specialized ||
             (Flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_DIELECTRIC_REFLECTION)) != 0) &&
            metalness < 1.0)
        {
            var reflected : float = 0;
            if (!Specialized || (Flow & FLOW_DIELECTRIC_REFLECTION) != 0)
            {
                let ior : float = material.surface.x / incident.x;
                reflected = primeFullOpenPbrMicrofacetDielectricMsCompensation(
                                energy, microfacet, cosine, primeFullOpenPbrIorAdjustment(ior, 1.0))
                                .z;
            }
            var base : float3 = 0;
            if (!Specialized || (Flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE)) != 0)
            {
                base = material.base_roughness.xyz;
                if (!Specialized || (Flow & FLOW_SUBSURFACE) != 0)
                {
                    let subsurface
                        : float =
                              Specialized && (Flow & FLOW_DIFFUSE) == 0 ? 1.0 : material.surface.z;
                    if (subsurface > 0.0)
                        base = lerp(base, primeFullPtSubsurfaceColor(base), subsurface);
                }
            }
            result.diffuse = (1.0 - metalness) * base * (1.0 - reflected);
            result.specular = (1.0 - metalness) * float3(reflected);
        }
        if ((!Specialized || (Flow & FLOW_CONDUCTOR_REFLECTION) != 0) && metalness > 0.0)
        {
            let f0 : float3 = saturate(material.base_roughness.xyz);
            let ess : float = primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet)
                                  ? 1.0
                                  : primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
                                        microfacet, cosine, float3(1), float3(1))
                                        .x;
            result.specular += metalness *
                               primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
                                   microfacet, cosine, f0, float3(1)) *
                               (float3(1) + f0 * (1.0 - ess) / ess);
        }
    }
    result.diffuse = primeSanitizeDenoiseAlbedo(result.diffuse);
    result.specular = primeSanitizeDenoiseAlbedo(result.specular);
    return result;
}

/// 任意合法材料的通用方向能量入口；不假设应用提供Flow键。
public func denoise_albedos<Energy>(energy: Energy, material: Material, normal: float3,
                                    view: float3, incident: float4, transmitted: float4)
    ->DenoiseAlbedos
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    return denoise_albedos_impl<0, false>(energy, material, normal, view, incident, transmitted);
}

/// Flow须由同一资源世代证明；删除无金属/无SSS/无反射/无透射的能量分支与读取。
/// 不改变允许的材料或数值精度，FLOW_METAL_MIX保留实际metalness与null proposal的权重。
public func denoise_albedos_flow<let Flow : uint, Energy>(energy: Energy, material: Material,
                                                          normal: float3, view: float3,
                                                          incident: float4, transmitted: float4)
    ->DenoiseAlbedos
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    if (openpbr_composition::HasSelectedFlow)
    {
        let value : openpbr_composition::GuideEnergy = openpbr_composition::Guide.directional(
                        energy, material, abs(dot(normal, view)), incident, transmitted);
        return { primeSanitizeDenoiseAlbedo(value.diffuse),
                 primeSanitizeDenoiseAlbedo(value.specular) };
    }
    else
    {
        return denoise_albedos_impl<Flow, true>(energy, material, normal, view, incident,
                                                transmitted);
    }
}

}
