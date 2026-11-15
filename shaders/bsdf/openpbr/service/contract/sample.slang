// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/contract/sample.slang";

public func primeGeometricReflectionSupport(geometricNormal: float3, viewDirection: float3,
                                            scatterDirection: float3)
    ->bool
{
    return dot(geometricNormal, viewDirection) * dot(geometricNormal, scatterDirection) > 0.0;
}

public func primeGeometricEventSupport(geometricNormal: float3, viewDirection: float3,
                                       scatterDirection: float3, eventFlags: uint)
    ->bool
{
    let reflection : bool = (eventFlags & PRIME_BSDF_EVENT_REFLECTION) != 0u;
    let transmission : bool = (eventFlags & PRIME_BSDF_EVENT_TRANSMISSION) != 0u;
    if (reflection == transmission)
    {
        return false;
    }
    let geometricSide
        : float = dot(geometricNormal, viewDirection) * dot(geometricNormal, scatterDirection);
    return reflection ? geometricSide > 0.0 : geometricSide < 0.0;
}

public static const let PRIME_PI = 3.14159265358979323846;

public static const let PRIME_BSDF_EVENT_REFLECTION : uint = 1 << 0;
public static const let PRIME_BSDF_EVENT_TRANSMISSION : uint = 1 << 1;
public static const let PRIME_BSDF_EVENT_DIFFUSE : uint = 1 << 2;
public static const let PRIME_BSDF_EVENT_GLOSSY : uint = 1 << 3;
public static const let PRIME_BSDF_EVENT_DISCRETE : uint = 1 << 4;
public static const let PRIME_MEASURE_SOLID_ANGLE : uint = 1;
public static const let PRIME_MEASURE_DISCRETE : uint = 2;

public struct PrimeClosureTraits
{
    public var measureMask : uint;
    public var eventMask : uint;
};

public func primeClosureSupports(traits: PrimeClosureTraits, measure: uint)->bool
{
    return (traits.measureMask & measure) != 0;
}

public func primeClosureIsDiscreteOnly(traits: PrimeClosureTraits)->bool
{
    return traits.measureMask == PRIME_MEASURE_DISCRETE;
}

public struct BsdfEvaluation
{
    public var response : float3;
    public var pdf : float;
};

public struct BsdfSample
{
    public var direction : float3;
    public var response : float3;
    public var pdf : float;
    public var relativeEta : float;
    public var eventFlags : uint;
};

public func primeGeometricSampleSupport(normalMapped: bool, geometricNormal: float3,
                                        viewDirection: float3, sample_value: BsdfSample)
    ->bool
{
    return !normalMapped ||
           primeGeometricEventSupport(geometricNormal, viewDirection, sample_value.direction,
                                      sample_value.eventFlags);
}

// Opaque direct lighting covers reflection only. Authored thin SSS transmission therefore
// has no competing NEE estimator, while dielectric direct lighting covers both sides.
public func primeDirectLightingMisPdf(dielectric: bool, sample_value: BsdfSample)->float
{
    let noCompetition
        : bool = (sample_value.eventFlags & PRIME_BSDF_EVENT_DISCRETE) != 0u ||
                 (!dielectric && (sample_value.eventFlags & PRIME_BSDF_EVENT_TRANSMISSION) != 0u);
    return noCompetition ? 0.0 : sample_value.pdf;
}

// Direct lighting uses the same physical boundary side as continuation. The source opaque
// direct-light contract accepts reflection only, including authored thin subsurface materials.
public func primeGeometricEvaluationSupport(normalMapped: bool, geometricNormal: float3,
                                            shadingNormal: float3, viewDirection: float3,
                                            scatterDirection: float3, dielectric: bool)
    ->bool
{
    if (!normalMapped)
        return true;
    let reflection
        : bool = primeGeometricReflectionSupport(geometricNormal, viewDirection, scatterDirection);
    let transmission
        : bool = primeGeometricEventSupport(geometricNormal, viewDirection, scatterDirection,
                                            PRIME_BSDF_EVENT_TRANSMISSION);
    if (!reflection && !transmission)
        return false;
    if (!dielectric)
        return reflection;
    let shadingSide : float =
                          dot(shadingNormal, viewDirection) * dot(shadingNormal, scatterDirection);
    return shadingSide != 0.0 && (shadingSide >= 0.0) == reflection;
}
