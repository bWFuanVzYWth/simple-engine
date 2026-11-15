/// RR方向能量的白名单组合；不构造BSDF采样状态。
module guide_composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.api;
import bsdf.openpbr.composition;
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/contract/sample.slang";

namespace openpbr_composition
{
public struct GuideEnergy
{
    public var diffuse : float3;
    public var specular : float3;
};
public interface IGuideEnergy
{
    static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy;
};
public struct BaseGuide<B : IBase> : IGuideEnergy
{
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        return { B.energy(float3(0, 0, cosine), B.prepare(m.base_roughness.xyz, m.surface.z)),
                 float3(0) };
    }
};
public struct DielectricGuide<B : IBase> : IGuideEnergy
{
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        let distribution
            : PrimeFullOpenPbrMicrofacet = { float2(m.base_roughness.w * m.base_roughness.w) };
        let reflected
            : float = primeFullOpenPbrMicrofacetDielectricMsCompensation<false, false, false, true>(
                          energy, distribution, cosine,
                          primeFullOpenPbrIorAdjustment(m.surface.x / incident.x, 1),
                          m.base_roughness.w)
                          .z;
        return { B.energy(float3(0, 0, cosine), B.prepare(m.base_roughness.xyz, m.surface.z)) *
                     (1 - reflected),
                 float3(reflected) };
    }
};
public struct MetalGuide : IGuideEnergy
{
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        let distribution
            : PrimeFullOpenPbrMicrofacet = { float2(m.base_roughness.w * m.base_roughness.w) };
        let f0 : float3 = saturate(m.base_roughness.xyz);
        let ab =
            primeFullOpenPbrMicrofacetDirectionalAlbedoCoefficients<true>(distribution, cosine);
        let ess : float =
                      primeFullOpenPbrMicrofacetEffectivelySmooth(distribution) ? 1 : ab.x + ab.y;
        return { float3(0), (f0 * ab.x + ab.y) * (float3(1) + f0 * (1 - ess) / ess) };
    }
};
public struct MetalGuideBlend<D : IGuideEnergy> : IGuideEnergy
{
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        var value : GuideEnergy = {};
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
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        if (m.surface.y == 1)
            return MetalGuide.directional(energy, m, cosine, incident, transmitted);
        return D.directional(energy, m, cosine, incident, transmitted);
    }
};
public struct SolidGuide : IGuideEnergy
{
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        let exiting : bool = incident.x > transmitted.x;
        let ior : float = exiting ? incident.x / transmitted.x : transmitted.x / incident.x;
        let distribution
            : PrimeFullOpenPbrMicrofacet = { float2(m.base_roughness.w * m.base_roughness.w) };
        let reflected
            : float = primeFullOpenPbrMicrofacetDielectricMsCompensation<false, false, false, true>(
                          energy, distribution, exiting ? -cosine : cosine, ior, m.base_roughness.w)
                          .z;
        return { float3(1 - reflected), float3(reflected) };
    }
};
public struct ThinGuide : IGuideEnergy
{
    public static func directional<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, m: openpbr::Material, cosine: float, incident: float4, transmitted: float4)
        ->GuideEnergy
    {
        let distribution
            : PrimeFullOpenPbrMicrofacet = { float2(m.base_roughness.w * m.base_roughness.w) };
        let reflected
            : float =
                  primeFullOpenPbrMicrofacetDielectricMsCompensation<false, false, false, true>(
                      energy, distribution, cosine, transmitted.x / incident.x, m.base_roughness.w)
                      .z;
        return { float3(1 - reflected), float3(reflected) };
    }
};

static const let GuideHasReflection : bool = (SelectedFlow & 2) != 0;
static const let GuideHasMetalMixture : bool = (SelectedFlow & 2048) != 0;
static const let GuideHasBinaryMetal : bool = (SelectedVariant & 2048) != 0;
static const let GuideHasMetal : bool = (SelectedFlow & 4) != 0;
static const let GuideIsSolid : bool = (SelectedFlow & 16) != 0;
static const let GuideIsThin : bool = (SelectedFlow & 32) != 0;
typealias DielectricGuidePart =
    TypeSelect<GuideHasReflection, DielectricGuide<SelectedBase>, BaseGuide<SelectedBase>>.Type;
typealias SwitchedGuide = MetalGuideSwitch<DielectricGuidePart>;
typealias BlendedGuide = MetalGuideBlend<DielectricGuidePart>;
typealias MixedGuide = TypeSelect<GuideHasBinaryMetal, SwitchedGuide, BlendedGuide>.Type;
typealias UnmixedGuide = TypeSelect<GuideHasMetal, MetalGuide, DielectricGuidePart>.Type;
typealias OpaqueGuide = TypeSelect<GuideHasMetalMixture, MixedGuide, UnmixedGuide>.Type;
typealias OtherGuide = TypeSelect<GuideIsThin, ThinGuide, OpaqueGuide>.Type;
public typealias Guide = TypeSelect<GuideIsSolid, SolidGuide, OtherGuide>.Type;
}
