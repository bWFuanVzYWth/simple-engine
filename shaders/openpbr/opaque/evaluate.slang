// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/opaque/evaluate.slang";

import openpbr.opaque.state;
import openpbr.opaque.lobes;
import openpbr.contract.flags;

namespace openpbr {
namespace kernel {
public func opaque_evaluate(wi: float3, wo: float3, state: OpaqueState)->LocalEvaluation {
    var result : LocalEvaluation;
    if (state.metalness > 0.0 && state.metalness < 1.0) {
        result.response = mix_eval(opaque_layer_eval(wi, wo, state, LOBE_ALL),
                                   conductor_eval(wi, wo, state, LOBE_ALL), state.metalness);
        result.pdf = lerp(opaque_layer_pdf(wi, wo, state), conductor_pdf(wi, wo, state, LOBE_ALL),
                          state.metalness);
        return result;
    }
    result.response = state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR
                          ? conductor_eval(wi, wo, state, LOBE_ALL)
                          : opaque_layer_eval(wi, wo, state, LOBE_ALL);
    result.pdf = state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR
                     ? conductor_pdf(wi, wo, state, LOBE_ALL)
                     : opaque_layer_pdf(wi, wo, state);
    return result;
}

public func opaque_evaluate_components(wi: float3, wo: float3, state: OpaqueState)
    ->OpaqueComponents {
    var result : OpaqueComponents;
    result.diffuse = float3(0.0);
    result.specular = float3(0.0);
    if (state.metalness > 0.0 && state.metalness < 1.0) {
        result.diffuse =
            (1.0 - state.metalness) * opaque_layer_eval(wi, wo, state, LOBE_DIFFUSE).value;
        result.specular = lerp(opaque_layer_eval(wi, wo, state, LOBE_SPECULAR | LOBE_DELTA).value,
                               conductor_eval(wi, wo, state, LOBE_ALL).value, state.metalness);
        result.pdf = opaque_evaluate(wi, wo, state).pdf;
        return result;
    }
    if (state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR) {
        result.specular = conductor_eval(wi, wo, state, LOBE_SPECULAR | LOBE_DELTA).value;
        result.pdf = conductor_pdf(wi, wo, state, LOBE_ALL);
        return result;
    }
    result.diffuse = opaque_layer_eval(wi, wo, state, LOBE_DIFFUSE).value;
    result.specular = opaque_layer_eval(wi, wo, state, LOBE_SPECULAR | LOBE_DELTA).value;
    result.pdf = opaque_layer_pdf(wi, wo, state);
    return result;
}
}
}
