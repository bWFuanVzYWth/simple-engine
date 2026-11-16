/// 只分类测度与方向支持，不建立BSDF状态或读取能量表。
module support_composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.api;
import bsdf.openpbr.math.spectrum;
namespace openpbr
{
namespace closure
{
public interface IDeltaDomain
{
    static func discrete(material: openpbr::Material, incident: float4, transmitted: float4)->bool;
};
public struct NeverDelta : IDeltaDomain
{
    public static func discrete(material: openpbr::Material, incident: float4, transmitted: float4)
        ->bool
    {
        return false;
    }
};
public struct AlwaysDelta : IDeltaDomain
{
    public static func discrete(material: openpbr::Material, incident: float4, transmitted: float4)
        ->bool
    {
        return true;
    }
};
public struct SolidDeltaDomain : IDeltaDomain
{
    public static func discrete(material: openpbr::Material, incident: float4, transmitted: float4)
        ->bool
    {
        let exiting : bool = incident.x > transmitted.x;
        let ior : float = (exiting ? incident.x : transmitted.x) /
                          (exiting ? transmitted.x : incident.x);
        return material.base_roughness.w * material.base_roughness.w < 1e-4 || ior == 1;
    }
};
public struct ThinDeltaDomain : IDeltaDomain
{
    public static func discrete(material: openpbr::Material, incident: float4, transmitted: float4)
        ->bool
    {
        return clamp(material.base_roughness.w * material.base_roughness.w *
                         kernel::thin_dielectric_roughness_scaler2(transmitted.x / incident.x),
                     0.0, 1.0) < 1e-4;
    }
};
public struct MetalDeltaDomain : IDeltaDomain
{
    public static func discrete(material: openpbr::Material, incident: float4, transmitted: float4)
        ->bool
    {
        return material.surface.y == 1 &&
               material.base_roughness.w * material.base_roughness.w < 1e-4;
    }
};
public interface IDirectionSupport
{
    static func allows(material: openpbr::Material, n: float3, evaluated: float3, d: float3)->bool;
};
public struct NoDirection : IDirectionSupport
{
    public static func allows(material: openpbr::Material, n: float3, evaluated: float3, d: float3)
        ->bool
    {
        return false;
    }
};
public struct BothDirections : IDirectionSupport
{
    public static func allows(material: openpbr::Material, n: float3, evaluated: float3, d: float3)
        ->bool
    {
        return true;
    }
};
public struct ReflectionDirection : IDirectionSupport
{
    public static func allows(material: openpbr::Material, n: float3, evaluated: float3, d: float3)
        ->bool
    {
        return !(dot(dot(n, evaluated) < 0 ? -n : n, d) < 0);
    }
};
public struct SmoothMetalMixture : IDirectionSupport
{
    public static func allows(material: openpbr::Material, n: float3, evaluated: float3, d: float3)
        ->bool
    {
        return material.surface.y != 1 && ReflectionDirection.allows(material, n, evaluated, d);
    }
};
public interface IBasePresence
{
    public static func present(material: openpbr::Material)->bool;
};
public struct AlwaysBase : IBasePresence
{
    public static func present(material: openpbr::Material)->bool
    {
        return true;
    }
};
public struct DielectricBase : IBasePresence
{
    public static func present(material: openpbr::Material)->bool
    {
        return material.surface.y < 1;
    }
};
public interface ISubsurfacePresence
{
    public static func present(material: openpbr::Material)->bool;
};
public struct FullSubsurface : ISubsurfacePresence
{
    public static func present(material: openpbr::Material)->bool
    {
        return true;
    }
};
public struct WeightedSubsurface : ISubsurfacePresence
{
    public static func present(material: openpbr::Material)->bool
    {
        return material.surface.z > 0;
    }
};
public struct SubsurfaceDirection<Base : IBasePresence, S : ISubsurfacePresence,
                                  R : IDirectionSupport> : IDirectionSupport
{
    public static func allows(material: openpbr::Material, n: float3, evaluated: float3, d: float3)
        ->bool
    {
        return (Base.present(material) && S.present(material)) ||
               R.allows(material, n, evaluated, d);
    }
};
static const let support_has_no_delta
    : bool = (selected_flow & (FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)) == 0 ||
             (selected_flow &
              (FLOW_CONDUCTOR_REFLECTION | FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) == 0;
static const let support_is_delta_only
    : bool = (selected_flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                               FLOW_CONTINUOUS_TRANSMISSION)) == 0 &&
             ((selected_flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) != 0 ||
              (selected_flow & FLOW_METAL_MIX) == 0);
static const let support_is_solid : bool = (selected_flow & FLOW_SOLID_TRANSMISSION) != 0;
static const let support_is_thin : bool = (selected_flow & FLOW_THIN_TRANSMISSION) != 0;
static const let support_is_transmission
    : bool = (selected_flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) != 0;
static const let support_has_metal_mixture : bool = (selected_flow & FLOW_METAL_MIX) != 0;
static const let support_has_smooth_metal_mixture
    : bool = support_has_metal_mixture && (selected_flow & FLOW_CONTINUOUS_REFLECTION) == 0;
static const let support_has_diffuse : bool = (selected_flow & FLOW_DIFFUSE) != 0;
static const let support_has_subsurface : bool = (selected_flow & FLOW_SUBSURFACE) != 0;
static const let support_has_no_continuous
    : bool = (selected_flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                               FLOW_CONTINUOUS_TRANSMISSION)) == 0;
typealias OtherDeltaDomain = TypeSelect<support_is_thin, ThinDeltaDomain, MetalDeltaDomain>.Type;
typealias MixedDeltaDomain = TypeSelect<support_is_solid, SolidDeltaDomain, OtherDeltaDomain>.Type;
typealias PresentDeltaDomain =
    TypeSelect<support_is_delta_only, AlwaysDelta, MixedDeltaDomain>.Type;
public typealias DeltaDomain =
    TypeSelect<support_has_no_delta, NeverDelta, PresentDeltaDomain>.Type;

typealias ReflectionSupport =
    TypeSelect<support_has_smooth_metal_mixture, SmoothMetalMixture, ReflectionDirection>.Type;
typealias BasePresence = TypeSelect<support_has_metal_mixture, DielectricBase, AlwaysBase>.Type;
typealias SubsurfacePresence =
    TypeSelect<support_has_diffuse, WeightedSubsurface, FullSubsurface>.Type;
typealias SubsurfaceSupport =
    SubsurfaceDirection<BasePresence, SubsurfacePresence, ReflectionSupport>;
typealias OpaqueSupport =
    TypeSelect<support_has_subsurface, SubsurfaceSupport, ReflectionSupport>.Type;
typealias ContinuousSupport =
    TypeSelect<support_is_transmission, BothDirections, OpaqueSupport>.Type;
public typealias DirectionSupport =
    TypeSelect<support_has_no_continuous, NoDirection, ContinuousSupport>.Type;
}
}
