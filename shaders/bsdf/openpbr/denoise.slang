/// OpenPBR 方向能量降噪辅助量；不包含照明、吞吐、发光或曝光。
module denoise;

import bsdf.openpbr.api;
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
    public float3 diffuse;
    public float3 specular;
};

/// 与生产闭包一致的方向能量，不是 baseColor 或 F0 的别名。
/// dielectric 的 diffuse 表示透射能量代理；不包含介质 Beer 或界面滤色。
/// 输入范围、能量表和方向约定同 api；normal 为单位表面法线，view 指向观察者。
public DenoiseAlbedos denoise_albedos<Energy>(Energy energy, Material material, float3 normal,
                                              float3 view, float4 incident, float4 transmitted)
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    bool dielectric = (material.control.x & 1u) != 0;
    bool thin = (material.control.x & 2u) != 0;
    bool exiting = dielectric && !thin && incident.x > transmitted.x;
    float cosine = abs(dot(normal, view)) * (exiting ? -1.0 : 1.0);
    PrimeFullOpenPbrMicrofacet microfacet;
    microfacet.alpha = float2(material.base_roughness.w * material.base_roughness.w);
    DenoiseAlbedos result = {};
    if (dielectric)
    {
        float ior = (exiting ? incident.x : transmitted.x) / (exiting ? transmitted.x : incident.x);
        float reflected = primeFullOpenPbrMicrofacetDielectricMsCompensation(
                              energy, microfacet, thin ? abs(cosine) : cosine, ior)
                              .z;
        result.diffuse = float3(1.0 - reflected);
        result.specular = float3(reflected);
    }
    else
    {
        float metalness = material.surface.y;
        if (metalness < 1.0)
        {
            float ior = material.surface.x / incident.x;
            float subsurface = material.surface.z;
            float reflected =
                primeFullOpenPbrMicrofacetDielectricMsCompensation(
                    energy, microfacet, cosine, primeFullOpenPbrIorAdjustment(ior, 1.0))
                    .z;
            float3 base = material.base_roughness.xyz;
            if (subsurface > 0.0)
                base = lerp(base, primeFullPtSubsurfaceColor(base), subsurface);
            result.diffuse = (1.0 - metalness) * base * (1.0 - reflected);
            result.specular = (1.0 - metalness) * float3(reflected);
        }
        if (metalness > 0.0)
        {
            float3 f0 = saturate(material.base_roughness.xyz);
            float ess = primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet)
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
}
