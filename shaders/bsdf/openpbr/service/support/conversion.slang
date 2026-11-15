// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/support/conversion.slang";

import "bsdf/openpbr/service/contract/sample.slang";
import "bsdf/openpbr/service/support/sanitization.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";

public func lessThan(first: float3, second: float3)->bool3
{
    return first < second;
}
public func lessThanEqual(first: float3, second: float3)->bool3
{
    return first <= second;
}

public func primeFullOpenPbrHasSample(sampleValue: PrimeFullOpenPbrSample)->bool
{
    if (sampleValue.throughput.flags == PRIME_FULL_OPENPBR_FLAG_NONE)
    {
        // A rejected proposal has no consumable payload. The adapter replaces it with the canonical
        // zero-event sample, so inspecting discarded fields only creates diagnostic false
        // positives.
        return false;
    }
    // relativeEta is sanitized after the adapter has accepted the internal proposal. Observe the
    // raw value here so that the sanitizer cannot erase the first numerical failure.
    if (!primeBsdfDirection(sampleValue.wo) || !primeBsdfFinite(sampleValue.throughput.value) ||
        !primeBsdfFinite(sampleValue.pdf) ||
        any(lessThan(sampleValue.throughput.value, float3(0.0))) || !(sampleValue.pdf > 0.0))
    {
        // Float singularities in the reference closure are contained at the adapter boundary.
        // A rejected proposal contributes zero and cannot poison later MIS or throughput state.
        return false;
    }
    return true;
}

public func primeFullOpenPbrToBsdfContinuousEventFlags(flags: uint)->uint
{
    var result = 0u;
    if (primeFullOpenPbrIsReflective(flags))
    {
        result |= PRIME_BSDF_EVENT_REFLECTION;
    }
    if (primeFullOpenPbrIsTransmissive(flags))
    {
        result |= PRIME_BSDF_EVENT_TRANSMISSION;
    }
    if (primeFullOpenPbrIsDiffuse(flags))
    {
        result |= PRIME_BSDF_EVENT_DIFFUSE;
    }
    if (primeFullOpenPbrIsSpecular(flags))
    {
        result |= PRIME_BSDF_EVENT_GLOSSY;
    }
    return result;
}

public func primeFullOpenPbrToBsdfEventFlags(flags: uint)->uint
{
    var result : uint = primeFullOpenPbrToBsdfContinuousEventFlags(flags);
    if (primeFullOpenPbrIsDelta(flags))
    {
        result |= PRIME_BSDF_EVENT_DISCRETE;
    }
    return result;
}
