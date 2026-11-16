// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/contract/sample.slang";

namespace openpbr
{
namespace kernel
{
public func geometric_reflection_support(geometric_normal: float3, view_direction: float3,
                                         scatter_direction: float3)
    ->bool
{
    return dot(geometric_normal, view_direction) * dot(geometric_normal, scatter_direction) > 0.0;
}

public func geometric_event_support(geometric_normal: float3, view_direction: float3,
                                    scatter_direction: float3, event_flags: uint)
    ->bool
{
    let reflection : bool = (event_flags & EVENT_REFLECTION) != 0u;
    let transmission : bool = (event_flags & EVENT_TRANSMISSION) != 0u;
    if (reflection == transmission)
    {
        return false;
    }
    let geometric_side
        : float = dot(geometric_normal, view_direction) * dot(geometric_normal, scatter_direction);
    return reflection ? geometric_side > 0.0 : geometric_side < 0.0;
}

public static const let EVENT_REFLECTION : uint = 1 << 0;
public static const let EVENT_TRANSMISSION : uint = 1 << 1;
public static const let EVENT_DIFFUSE : uint = 1 << 2;
public static const let EVENT_GLOSSY : uint = 1 << 3;
public static const let EVENT_DISCRETE : uint = 1 << 4;
public static const let MEASURE_SOLID_ANGLE : uint = 1;
public static const let MEASURE_DISCRETE : uint = 2;

public struct ClosureTraits
{
    public var measure_mask : uint;
    public var event_mask : uint;
};

public func closure_supports(traits: ClosureTraits, measure: uint)->bool
{
    return (traits.measure_mask & measure) != 0;
}

public func closure_is_discrete_only(traits: ClosureTraits)->bool
{
    return traits.measure_mask == MEASURE_DISCRETE;
}

public struct SurfaceEvaluation
{
    public var response : float3;
    public var pdf : float;
};

public struct SurfaceSample
{
    public var direction : float3;
    public var response : float3;
    public var pdf : float;
    public var relative_eta : float;
    public var event_flags : uint;
};

public func geometric_sample_support(normal_mapped: bool, geometric_normal: float3,
                                     view_direction: float3, sample_value: SurfaceSample)
    ->bool
{
    return !normal_mapped ||
           geometric_event_support(geometric_normal, view_direction, sample_value.direction,
                                   sample_value.event_flags);
}

// Opaque direct lighting covers reflection only. Authored thin SSS transmission therefore
// has no competing NEE estimator, while dielectric direct lighting covers both sides.
public func direct_lighting_mis_pdf(dielectric: bool, sample_value: SurfaceSample)->float
{
    let no_competition : bool =
                             (sample_value.event_flags & EVENT_DISCRETE) != 0u ||
                             (!dielectric && (sample_value.event_flags & EVENT_TRANSMISSION) != 0u);
    return no_competition ? 0.0 : sample_value.pdf;
}

// Direct lighting uses the same physical boundary side as continuation. The source opaque
// direct-light contract accepts reflection only, including authored thin subsurface materials.
public func geometric_evaluation_support(normal_mapped: bool, geometric_normal: float3,
                                         shading_normal: float3, view_direction: float3,
                                         scatter_direction: float3, dielectric: bool)
    ->bool
{
    if (!normal_mapped)
        return true;
    let reflection
        : bool = geometric_reflection_support(geometric_normal, view_direction, scatter_direction);
    let transmission : bool = geometric_event_support(geometric_normal, view_direction,
                                                      scatter_direction, EVENT_TRANSMISSION);
    if (!reflection && !transmission)
        return false;
    if (!dielectric)
        return reflection;
    let shading_side
        : float = dot(shading_normal, view_direction) * dot(shading_normal, scatter_direction);
    return shading_side != 0.0 && (shading_side >= 0.0) == reflection;
}
}
}
