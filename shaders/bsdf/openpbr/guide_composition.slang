/// RR方向能量的白名单组合；不构造BSDF采样状态。
module guide_composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.api;
import bsdf.openpbr.composition;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.math.microfacet_measure;
import bsdf.openpbr.math.spectrum;
import bsdf.openpbr.contract.sample;

namespace openpbr
{
namespace closure
{
public struct GuideEnergy
{
    public var diffuse : float3;
    public var specular : float3;
};
public interface IGuideEnergy
{
    static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut, material: openpbr::Material,
                                                    cosine: float, incident: float4,
                                                    transmitted: float4)
        ->GuideEnergy;
};
public struct BaseGuide<Base : IBase> : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        return { Base.energy(float3(0, 0, cosine),
                             Base.prepare(material.base_roughness.xyz, material.surface.z)),
                 float3(0) };
    }
};
public struct DielectricGuide<Base : IBase> : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        let distribution : kernel::Microfacet = { float2(material.base_roughness.w *
                                                         material.base_roughness.w) };
        let reflected : float = kernel::dielectric_energy<false, false, false, true>(
                                    energy, distribution, cosine,
                                    kernel::ior_adjustment(material.surface.x / incident.x, 1),
                                    material.base_roughness.w)
                                    .z;
        return { Base.energy(float3(0, 0, cosine),
                             Base.prepare(material.base_roughness.xyz, material.surface.z)) *
                     (1 - reflected),
                 float3(reflected) };
    }
};
public struct MetalGuide : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        let distribution : kernel::Microfacet = { float2(material.base_roughness.w *
                                                         material.base_roughness.w) };
        let f0 : float3 = saturate(material.base_roughness.xyz);
        let ab = kernel::microfacet_directional_albedo_coefficients<true>(distribution, cosine);
        let ess : float = kernel::microfacet_effectively_smooth(distribution) ? 1 : ab.x + ab.y;
        return { float3(0), (f0 * ab.x + ab.y) * (float3(1) + f0 * (1 - ess) / ess) };
    }
};
public struct MetalGuideBlend<Dielectric : IGuideEnergy> : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        var value : GuideEnergy = {};
        if (material.surface.y < 1)
        {
            value = Dielectric.directional(energy, material, cosine, incident, transmitted);
            value.diffuse *= 1 - material.surface.y;
            value.specular *= 1 - material.surface.y;
        }
        if (material.surface.y > 0)
            value.specular +=
                material.surface.y *
                MetalGuide.directional(energy, material, cosine, incident, transmitted).specular;
        return value;
    }
};
public struct MetalGuideSwitch<Dielectric : IGuideEnergy> : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        if (material.surface.y == 1)
            return MetalGuide.directional(energy, material, cosine, incident, transmitted);
        return Dielectric.directional(energy, material, cosine, incident, transmitted);
    }
};
public struct SolidGuide : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        let exiting : bool = incident.x > transmitted.x;
        let ior : float = exiting ? incident.x / transmitted.x : transmitted.x / incident.x;
        let distribution : kernel::Microfacet = { float2(material.base_roughness.w *
                                                         material.base_roughness.w) };
        let reflected : float = kernel::dielectric_energy<false, false, false, true>(
                                    energy, distribution, exiting ? -cosine : cosine, ior,
                                    material.base_roughness.w)
                                    .z;
        return { float3(1 - reflected), float3(reflected) };
    }
};
public struct ThinGuide : IGuideEnergy
{
    public static func directional<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                           material: openpbr::Material,
                                                           cosine: float, incident: float4,
                                                           transmitted: float4)
        ->GuideEnergy
    {
        let distribution : kernel::Microfacet = { float2(material.base_roughness.w *
                                                         material.base_roughness.w) };
        let reflected : float = kernel::dielectric_energy<false, false, false, true>(
                                    energy, distribution, cosine, transmitted.x / incident.x,
                                    material.base_roughness.w)
                                    .z;
        return { float3(1 - reflected), float3(reflected) };
    }
};

static const let guide_has_reflection : bool = (selected_flow & FLOW_DIELECTRIC_REFLECTION) != 0;
static const let guide_has_metal_mixture : bool = (selected_flow & FLOW_METAL_MIX) != 0;
static const let guide_has_binary_metal : bool = (selected_variant & variant_metal_switch) != 0;
static const let guide_has_metal : bool = (selected_flow & FLOW_CONDUCTOR_REFLECTION) != 0;
static const let guide_is_solid : bool = (selected_flow & FLOW_SOLID_TRANSMISSION) != 0;
static const let guide_is_thin : bool = (selected_flow & FLOW_THIN_TRANSMISSION) != 0;
typealias DielectricGuidePart =
    TypeSelect<guide_has_reflection, DielectricGuide<SelectedBase>, BaseGuide<SelectedBase>>.Type;
typealias SwitchedGuide = MetalGuideSwitch<DielectricGuidePart>;
typealias BlendedGuide = MetalGuideBlend<DielectricGuidePart>;
typealias MixedGuide = TypeSelect<guide_has_binary_metal, SwitchedGuide, BlendedGuide>.Type;
typealias UnmixedGuide = TypeSelect<guide_has_metal, MetalGuide, DielectricGuidePart>.Type;
typealias OpaqueGuide = TypeSelect<guide_has_metal_mixture, MixedGuide, UnmixedGuide>.Type;
typealias OtherGuide = TypeSelect<guide_is_thin, ThinGuide, OpaqueGuide>.Type;
public typealias Guide = TypeSelect<guide_is_solid, SolidGuide, OtherGuide>.Type;
}
}
