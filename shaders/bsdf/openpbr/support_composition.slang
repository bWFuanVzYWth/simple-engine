/// 只分类测度与方向支持，不建立BSDF状态或读取能量表。
module support_composition;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/spectrum.slang";
namespace openpbr_composition
{
public interface IDeltaDomain
{
    static bool discrete(openpbr::Material m, float4 incident, float4 transmitted);
};
public struct NeverDelta : IDeltaDomain
{
    public static bool discrete(openpbr::Material m, float4 incident, float4 transmitted)
    {
        return false;
    }
};
public struct AlwaysDelta : IDeltaDomain
{
    public static bool discrete(openpbr::Material m, float4 incident, float4 transmitted)
    {
        return true;
    }
};
public struct SolidDeltaDomain : IDeltaDomain
{
    public static bool discrete(openpbr::Material m, float4 incident, float4 transmitted)
    {
        bool exiting = incident.x > transmitted.x;
        float ior = (exiting ? incident.x : transmitted.x) / (exiting ? transmitted.x : incident.x);
        return m.base_roughness.w * m.base_roughness.w < 1e-4 || ior == 1;
    }
};
public struct ThinDeltaDomain : IDeltaDomain
{
    public static bool discrete(openpbr::Material m, float4 incident, float4 transmitted)
    {
        return clamp(m.base_roughness.w * m.base_roughness.w *
                         primeFullOpenPbrThinDielectricRoughnessScaler2(transmitted.x / incident.x),
                     0.0, 1.0) < 1e-4;
    }
};
public struct MetalDeltaDomain : IDeltaDomain
{
    public static bool discrete(openpbr::Material m, float4 incident, float4 transmitted)
    {
        return m.surface.y == 1 && m.base_roughness.w * m.base_roughness.w < 1e-4;
    }
};
public interface IDirectionSupport
{
    static bool allows(openpbr::Material m, float3 n, float3 v, float3 d);
};
public struct NoDirection : IDirectionSupport
{
    public static bool allows(openpbr::Material m, float3 n, float3 v, float3 d)
    {
        return false;
    }
};
public struct BothDirections : IDirectionSupport
{
    public static bool allows(openpbr::Material m, float3 n, float3 v, float3 d)
    {
        return true;
    }
};
public struct ReflectionDirection : IDirectionSupport
{
    public static bool allows(openpbr::Material m, float3 n, float3 v, float3 d)
    {
        return !(dot(dot(n, v) < 0 ? -n : n, d) < 0);
    }
};
public struct SmoothMetalMixture : IDirectionSupport
{
    public static bool allows(openpbr::Material m, float3 n, float3 v, float3 d)
    {
        return m.surface.y != 1 && ReflectionDirection.allows(m, n, v, d);
    }
};
public interface IBasePresence
{
    public static bool present(openpbr::Material m);
};
public struct AlwaysBase : IBasePresence
{
    public static bool present(openpbr::Material m)
    {
        return true;
    }
};
public struct DielectricBase : IBasePresence
{
    public static bool present(openpbr::Material m)
    {
        return m.surface.y < 1;
    }
};
public interface ISubsurfacePresence
{
    public static bool present(openpbr::Material m);
};
public struct FullSubsurface : ISubsurfacePresence
{
    public static bool present(openpbr::Material m)
    {
        return true;
    }
};
public struct WeightedSubsurface : ISubsurfacePresence
{
    public static bool present(openpbr::Material m)
    {
        return m.surface.z > 0;
    }
};
public struct SubsurfaceDirection<B : IBasePresence, S : ISubsurfacePresence, R : IDirectionSupport>
    : IDirectionSupport
{
    public static bool allows(openpbr::Material m, float3 n, float3 v, float3 d)
    {
        return (B.present(m) && S.present(m)) || R.allows(m, n, v, d);
    }
};
#if defined(OPENPBR_FLOW)
#define SUPPORT_FLOW OPENPBR_FLOW
#elif defined(GAME_FLOW)
#define SUPPORT_FLOW GAME_FLOW
#endif
#ifdef SUPPORT_FLOW
#if !(SUPPORT_FLOW & 320) || !(SUPPORT_FLOW & 52)
public typedef NeverDelta DeltaDomain;
#elif !(SUPPORT_FLOW & 649) && ((SUPPORT_FLOW & 48) || !(SUPPORT_FLOW & 2048))
public typedef AlwaysDelta DeltaDomain;
#elif SUPPORT_FLOW & 16
public typedef SolidDeltaDomain DeltaDomain;
#elif SUPPORT_FLOW & 32
public typedef ThinDeltaDomain DeltaDomain;
#else
public typedef MetalDeltaDomain DeltaDomain;
#endif
#if (SUPPORT_FLOW & 2048) && !(SUPPORT_FLOW & 128)
typedef SmoothMetalMixture ReflectionSupport;
#else
typedef ReflectionDirection ReflectionSupport;
#endif
#if SUPPORT_FLOW & 2048
typedef DielectricBase BasePresence;
#else
typedef AlwaysBase BasePresence;
#endif
#if SUPPORT_FLOW & 1
typedef WeightedSubsurface SubsurfacePresence;
#else
typedef FullSubsurface SubsurfacePresence;
#endif
#if !(SUPPORT_FLOW & 649)
public typedef NoDirection DirectionSupport;
#elif SUPPORT_FLOW & 48
public typedef BothDirections DirectionSupport;
#elif SUPPORT_FLOW & 8
public typedef SubsurfaceDirection<BasePresence, SubsurfacePresence, ReflectionSupport>
    DirectionSupport;
#else
public typedef ReflectionSupport DirectionSupport;
#endif
#endif
}
