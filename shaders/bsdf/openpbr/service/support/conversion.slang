// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/support/conversion.slang";

import bsdf.openpbr.service.contract.sample;
import bsdf.openpbr.service.support.sanitization;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.math.frame;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.contract.flags;

namespace openpbr
{
namespace kernel
{
public func less_than(first: float3, second: float3)->bool3
{
    return first < second;
}
public func less_than_equal(first: float3, second: float3)->bool3
{
    return first <= second;
}

public func has_sample(sample_value: Proposal)->bool
{
    if (sample_value.response.flags == LOBE_NONE)
    {
        // A rejected proposal has no consumable payload. The adapter replaces it with the canonical
        // zero-event sample, so inspecting discarded fields only creates diagnostic false
        // positives.
        return false;
    }
    // relative_eta is sanitized after the adapter has accepted the internal proposal. Observe the
    // raw value here so that the sanitizer cannot erase the first numerical failure.
    if (!is_direction(sample_value.wo) || !is_finite(sample_value.response.value) ||
        !is_finite(sample_value.pdf) || any(less_than(sample_value.response.value, float3(0.0))) ||
        !(sample_value.pdf > 0.0))
    {
        // Float singularities in the reference closure are contained at the adapter boundary.
        // A rejected proposal contributes zero and cannot poison later MIS or response state.
        return false;
    }
    return true;
}

public func to_bsdf_continuous_event_flags(flags: uint)->uint
{
    var result = 0u;
    if (is_reflective(flags))
    {
        result |= EVENT_REFLECTION;
    }
    if (is_transmissive(flags))
    {
        result |= EVENT_TRANSMISSION;
    }
    if (is_diffuse(flags))
    {
        result |= EVENT_DIFFUSE;
    }
    if (is_specular(flags))
    {
        result |= EVENT_GLOSSY;
    }
    return result;
}

public func to_bsdf_event_flags(flags: uint)->uint
{
    var result : uint = to_bsdf_continuous_event_flags(flags);
    if (is_delta(flags))
    {
        result |= EVENT_DISCRETE;
    }
    return result;
}
}
}
