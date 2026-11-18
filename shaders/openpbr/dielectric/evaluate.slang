// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/dielectric/evaluate.slang";

import openpbr.dielectric.contract;
import openpbr.dielectric.refract;
import openpbr.dielectric.state;

namespace openpbr {
namespace kernel {
public func transmission_evaluate(wi: float3, wo: float3, state: TransmissionState,
                                  sampling_flags: uint)
    ->TransmissionEvaluation {
    let closure : TransmissionState = transmission_closure_state(state);
    return transmission_evaluate_raw(wi, wo, closure, sampling_flags);
}

public func transmission_evaluate_solid_continuous(wi: float3, wo: float3, state: TransmissionState,
                                                   sampling_flags: uint)
    ->TransmissionEvaluation {
    let closure : TransmissionState = transmission_closure_state(state);
    return transmission_evaluate_raw_solid_continuous(wi, wo, closure, sampling_flags);
}
}
}
