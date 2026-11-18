// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/service/sanitization.slang";

import openpbr.service.sample;
import openpbr.service.albedo;
import openpbr.service.components;

namespace openpbr {
namespace kernel {
public func invalid_surface_evaluation()->SurfaceEvaluation {
    var result : SurfaceEvaluation;
    result.response = float3(0.0);
    result.pdf = 0.0;
    return result;
}

public func invalid_surface_sample()->SurfaceSample {
    var result : SurfaceSample;
    result.direction = float3(0.0);
    result.response = float3(0.0);
    result.pdf = 0.0;
    result.relative_eta = 1.0;
    result.event_flags = 0;
    return result;
}

public func is_finite(value: float)->bool {
    return !isnan(value) && !isinf(value);
}

public func is_finite(value: float3)->bool {
    return !any(isnan(value)) && !any(isinf(value));
}

public func is_direction(value: float3)->bool {
    if (!is_finite(value))
        return false;
    let length_squared : float = dot(value, value);
    return is_finite(length_squared) && length_squared > 1.0e-12 &&
           abs(length_squared - 1.0) <= 1.0e-3;
}

public func sanitize_surface_evaluation(evaluation: SurfaceEvaluation)->SurfaceEvaluation {
    if (!is_finite(evaluation.response) || !is_finite(evaluation.pdf) ||
        any(evaluation.response < float3(0.0)) || evaluation.pdf < 0.0) {
        return invalid_surface_evaluation();
    }
    return evaluation;
}

public func sanitize_surface_sample<let unit_eta : bool = false>(sample_value: SurfaceSample)
    ->SurfaceSample {
    if (sample_value.event_flags == 0) {
        return invalid_surface_sample();
    }
    return sanitize_surface_sample_with_events<unit_eta>(sample_value);
}

/// 调用方已证明 event_flags 非零；UnitEta=true 另外保证 relative_eta=1。
/// 仍检查方向、响应与 PDF；未证明的 eta 保持原数值检查。
public func sanitize_surface_sample_with_events<let unit_eta : bool = false>(
    sample_value: SurfaceSample)
    ->SurfaceSample {
    if (!is_direction(sample_value.direction) || !is_finite(sample_value.response) ||
        !is_finite(sample_value.pdf) || (!unit_eta && !is_finite(sample_value.relative_eta)) ||
        any(sample_value.response < float3(0.0)) || !(sample_value.pdf > 0.0) ||
        (!unit_eta && !(sample_value.relative_eta > 0.0))) {
        return invalid_surface_sample();
    }
    return sample_value;
}

public func sanitize_denoise_albedo(albedo: float3)->float3 {
    return float3(is_finite(albedo.x) ? clamp(albedo.x, 0.0, 1.0) : 0.0,
                  is_finite(albedo.y) ? clamp(albedo.y, 0.0, 1.0) : 0.0,
                  is_finite(albedo.z) ? clamp(albedo.z, 0.0, 1.0) : 0.0);
}

public func sanitize_surface_components(components: SurfaceComponents)->SurfaceComponents {
    var components = components;
    if (!is_finite(components.diffuse_response) || !is_finite(components.specular_response) ||
        !is_finite(components.pdf) || any(components.diffuse_response < float3(0.0)) ||
        any(components.specular_response < float3(0.0)) || components.pdf < 0.0) {
        components.diffuse_response = float3(0.0);
        components.specular_response = float3(0.0);
        components.pdf = 0.0;
    }
    return components;
}
}
}
