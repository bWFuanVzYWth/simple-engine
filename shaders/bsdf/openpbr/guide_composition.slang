/// RR方向能量的白名单组合；不构造BSDF采样状态。
module guide_composition;
import bsdf.openpbr.api;
import bsdf.openpbr.composition;
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/contract/sample.slang";

#ifdef OPENPBR_RECIPE_VARIANT
#define GUIDE_VARIANT OPENPBR_VARIANT
#else
#define GUIDE_VARIANT 0
#endif

namespace openpbr_composition
{
public struct GuideEnergy
{
    public float3 diffuse;
    public float3 specular;
};
public interface IGuideEnergy
{
    static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted);
};
public struct BaseGuide<B : IBase> : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        return { B.energy(float3(0, 0, cosine), B.prepare(m.base_roughness.xyz, m.surface.z)),
                 float3(0) };
    }
};
public struct DielectricGuide<B : IBase> : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        PrimeFullOpenPbrMicrofacet distribution = { float2(m.base_roughness.w *
                                                           m.base_roughness.w) };
        float reflected = primeFullOpenPbrMicrofacetDielectricMsCompensation(
                              energy, distribution, cosine,
                              primeFullOpenPbrIorAdjustment(m.surface.x / incident.x, 1))
                              .z;
        return { B.energy(float3(0, 0, cosine), B.prepare(m.base_roughness.xyz, m.surface.z)) *
                     (1 - reflected),
                 float3(reflected) };
    }
};
public struct MetalGuide : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        PrimeFullOpenPbrMicrofacet distribution = { float2(m.base_roughness.w *
                                                           m.base_roughness.w) };
        float3 f0 = saturate(m.base_roughness.xyz);
        float ess = primeFullOpenPbrMicrofacetEffectivelySmooth(distribution)
                        ? 1
                        : primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
                              distribution, cosine, float3(1), float3(1))
                              .x;
        return { float3(0), primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
                                distribution, cosine, f0, float3(1)) *
                                (float3(1) + f0 * (1 - ess) / ess) };
    }
};
public struct MetalGuideBlend<D : IGuideEnergy> : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        GuideEnergy value = {};
        if (m.surface.y < 1)
        {
            value = D.directional(energy, m, cosine, incident, transmitted);
            value.diffuse *= 1 - m.surface.y;
            value.specular *= 1 - m.surface.y;
        }
        if (m.surface.y > 0)
            value.specular +=
                m.surface.y *
                MetalGuide.directional(energy, m, cosine, incident, transmitted).specular;
        return value;
    }
};
public struct MetalGuideSwitch<D : IGuideEnergy> : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        if (m.surface.y == 1)
            return MetalGuide.directional(energy, m, cosine, incident, transmitted);
        return D.directional(energy, m, cosine, incident, transmitted);
    }
};
public struct SolidGuide : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        bool exiting = incident.x > transmitted.x;
        float ior = exiting ? incident.x / transmitted.x : transmitted.x / incident.x;
        PrimeFullOpenPbrMicrofacet distribution = { float2(m.base_roughness.w *
                                                           m.base_roughness.w) };
        float reflected = primeFullOpenPbrMicrofacetDielectricMsCompensation(
                              energy, distribution, exiting ? -cosine : cosine, ior)
                              .z;
        return { float3(1 - reflected), float3(reflected) };
    }
};
public struct ThinGuide : IGuideEnergy
{
    public static GuideEnergy directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        P energy, openpbr::Material m, float cosine, float4 incident, float4 transmitted)
    {
        PrimeFullOpenPbrMicrofacet distribution = { float2(m.base_roughness.w *
                                                           m.base_roughness.w) };
        float reflected = primeFullOpenPbrMicrofacetDielectricMsCompensation(
                              energy, distribution, cosine, transmitted.x / incident.x)
                              .z;
        return { float3(1 - reflected), float3(reflected) };
    }
};

#if defined(OPENPBR_FLOW)
#define GUIDE_FLOW OPENPBR_FLOW
#elif defined(GAME_FLOW)
#define GUIDE_FLOW GAME_FLOW
#endif
#ifdef GUIDE_FLOW
#if GUIDE_FLOW & 16
public typedef SolidGuide Guide;
#elif GUIDE_FLOW & 32
public typedef ThinGuide Guide;
#else
#if GUIDE_FLOW & 2
typedef DielectricGuide<SelectedBase> DielectricGuidePart;
#else
typedef BaseGuide<SelectedBase> DielectricGuidePart;
#endif
#if GUIDE_FLOW & 2048
#if (GUIDE_VARIANT & 2048)
public typedef MetalGuideSwitch<DielectricGuidePart> Guide;
#else
public typedef MetalGuideBlend<DielectricGuidePart> Guide;
#endif
#elif GUIDE_FLOW & 4
public typedef MetalGuide Guide;
#else
public typedef DielectricGuidePart Guide;
#endif
#endif
#endif
}
