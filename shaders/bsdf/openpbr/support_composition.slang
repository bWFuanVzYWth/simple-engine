/// 只分类测度与方向支持，不建立BSDF状态或读取能量表。
module support_composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/spectrum.slang";
namespace openpbr_composition
{
public interface IDeltaDomain
{
    static func discrete(m: openpbr::Material, incident: float4, transmitted: float4)->bool;
};
public struct NeverDelta : IDeltaDomain
{
    public static func discrete(m: openpbr::Material, incident: float4, transmitted: float4)->bool
    {
        return false;
    }
};
public struct AlwaysDelta : IDeltaDomain
{
    public static func discrete(m: openpbr::Material, incident: float4, transmitted: float4)->bool
    {
        return true;
    }
};
public struct SolidDeltaDomain : IDeltaDomain
{
    public static func discrete(m: openpbr::Material, incident: float4, transmitted: float4)->bool
    {
        let exiting : bool = incident.x > transmitted.x;
        let ior : float = (exiting ? incident.x : transmitted.x) /
                          (exiting ? transmitted.x : incident.x);
        return m.base_roughness.w * m.base_roughness.w < 1e-4 || ior == 1;
    }
};
public struct ThinDeltaDomain : IDeltaDomain
{
    public static func discrete(m: openpbr::Material, incident: float4, transmitted: float4)->bool
    {
        return clamp(m.base_roughness.w * m.base_roughness.w *
                         primeFullOpenPbrThinDielectricRoughnessScaler2(transmitted.x / incident.x),
                     0.0, 1.0) < 1e-4;
    }
};
public struct MetalDeltaDomain : IDeltaDomain
{
    public static func discrete(m: openpbr::Material, incident: float4, transmitted: float4)->bool
    {
        return m.surface.y == 1 && m.base_roughness.w * m.base_roughness.w < 1e-4;
    }
};
public interface IDirectionSupport
{
    static func allows(m: openpbr::Material, n: float3, v: float3, d: float3)->bool;
};
public struct NoDirection : IDirectionSupport
{
    public static func allows(m: openpbr::Material, n: float3, v: float3, d: float3)->bool
    {
        return false;
    }
};
public struct BothDirections : IDirectionSupport
{
    public static func allows(m: openpbr::Material, n: float3, v: float3, d: float3)->bool
    {
        return true;
    }
};
public struct ReflectionDirection : IDirectionSupport
{
    public static func allows(m: openpbr::Material, n: float3, v: float3, d: float3)->bool
    {
        return !(dot(dot(n, v) < 0 ? -n : n, d) < 0);
    }
};
public struct SmoothMetalMixture : IDirectionSupport
{
    public static func allows(m: openpbr::Material, n: float3, v: float3, d: float3)->bool
    {
        return m.surface.y != 1 && ReflectionDirection.allows(m, n, v, d);
    }
};
public interface IBasePresence
{
    public static func present(m: openpbr::Material)->bool;
};
public struct AlwaysBase : IBasePresence
{
    public static func present(m: openpbr::Material)->bool
    {
        return true;
    }
};
public struct DielectricBase : IBasePresence
{
    public static func present(m: openpbr::Material)->bool
    {
        return m.surface.y < 1;
    }
};
public interface ISubsurfacePresence
{
    public static func present(m: openpbr::Material)->bool;
};
public struct FullSubsurface : ISubsurfacePresence
{
    public static func present(m: openpbr::Material)->bool
    {
        return true;
    }
};
public struct WeightedSubsurface : ISubsurfacePresence
{
    public static func present(m: openpbr::Material)->bool
    {
        return m.surface.z > 0;
    }
};
public struct SubsurfaceDirection<B : IBasePresence, S : ISubsurfacePresence, R : IDirectionSupport>
    : IDirectionSupport
{
    public static func allows(m: openpbr::Material, n: float3, v: float3, d: float3)->bool
    {
        return (B.present(m) && S.present(m)) || R.allows(m, n, v, d);
    }
};
static const let SupportHasNoDelta : bool = (SelectedFlow & 320) == 0 || (SelectedFlow & 52) == 0;
static const let SupportIsDeltaOnly
    : bool = (SelectedFlow & 649) == 0 && ((SelectedFlow & 48) != 0 || (SelectedFlow & 2048) == 0);
static const let SupportIsSolid : bool = (SelectedFlow & 16) != 0;
static const let SupportIsThin : bool = (SelectedFlow & 32) != 0;
static const let SupportIsTransmission : bool = (SelectedFlow & 48) != 0;
static const let SupportHasMetalMixture : bool = (SelectedFlow & 2048) != 0;
static const let SupportHasSmoothMetalMixture
    : bool = SupportHasMetalMixture && (SelectedFlow & 128) == 0;
static const let SupportHasDiffuse : bool = (SelectedFlow & 1) != 0;
static const let SupportHasSubsurface : bool = (SelectedFlow & 8) != 0;
static const let SupportHasNoContinuous : bool = (SelectedFlow & 649) == 0;
typealias OtherDeltaDomain = TypeSelect<SupportIsThin, ThinDeltaDomain, MetalDeltaDomain>.Type;
typealias MixedDeltaDomain = TypeSelect<SupportIsSolid, SolidDeltaDomain, OtherDeltaDomain>.Type;
typealias PresentDeltaDomain = TypeSelect<SupportIsDeltaOnly, AlwaysDelta, MixedDeltaDomain>.Type;
public typealias DeltaDomain = TypeSelect<SupportHasNoDelta, NeverDelta, PresentDeltaDomain>.Type;

typealias ReflectionSupport =
    TypeSelect<SupportHasSmoothMetalMixture, SmoothMetalMixture, ReflectionDirection>.Type;
typealias BasePresence = TypeSelect<SupportHasMetalMixture, DielectricBase, AlwaysBase>.Type;
typealias SubsurfacePresence =
    TypeSelect<SupportHasDiffuse, WeightedSubsurface, FullSubsurface>.Type;
typealias SubsurfaceSupport =
    SubsurfaceDirection<BasePresence, SubsurfacePresence, ReflectionSupport>;
typealias OpaqueSupport =
    TypeSelect<SupportHasSubsurface, SubsurfaceSupport, ReflectionSupport>.Type;
typealias ContinuousSupport = TypeSelect<SupportIsTransmission, BothDirections, OpaqueSupport>.Type;
public typealias DirectionSupport =
    TypeSelect<SupportHasNoContinuous, NoDirection, ContinuousSupport>.Type;
}
