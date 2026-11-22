// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/service/conversion.slang";

import openpbr.service.sample;
import openpbr.service.sanitization;
import openpbr.contract.material;
import openpbr.math.frame;
import openpbr.contract.sample;
import openpbr.contract.flags;

namespace openpbr {
namespace kernel {
public func less_than(first: float3, second: float3)->bool3 {
    return first < second;
}
public func less_than_equal(first: float3, second: float3)->bool3 {
    return first <= second;
}

public func has_sample(sample_value: Proposal)->bool {
    if (sample_value.response.flags == LOBE_NONE) {
        // A rejected proposal has no consumable payload. The adapter replaces it with the canonical
        // zero-event sample, so inspecting discarded fields only creates diagnostic false
        // positives.
        return false;
    }
    // relative_eta is sanitized after the adapter has accepted the internal proposal. Observe the
    // raw value here so that the sanitizer cannot erase the first numerical failure.
    if (!is_direction(sample_value.wo) || !is_finite(sample_value.response.value) ||
        !is_finite(sample_value.pdf) || any(less_than(sample_value.response.value, float3(0.0))) ||
        !(sample_value.pdf > 0.0)) {
        // Float singularities in the reference closure are contained at the adapter boundary.
        // A rejected proposal contributes zero and cannot poison later MIS or response state.
        return false;
    }
    return true;
}

public func to_bsdf_continuous_event_flags(flags: uint)->uint {
    return (is_reflective(flags) ? EVENT_REFLECTION : 0u) |
           (is_transmissive(flags) ? EVENT_TRANSMISSION : 0u) |
           (is_diffuse(flags) ? EVENT_DIFFUSE : 0u) | (is_specular(flags) ? EVENT_GLOSSY : 0u);
}

public func to_bsdf_event_flags(flags: uint)->uint {
    return to_bsdf_continuous_event_flags(flags) | (is_delta(flags) ? EVENT_DISCRETE : 0u);
}
}
}
