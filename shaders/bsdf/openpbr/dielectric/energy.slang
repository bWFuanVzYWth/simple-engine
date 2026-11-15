// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/energy.slang";

import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";

public func primeFullCompactTransmissionEnergy<TransmissionGgxEnergy>(
    transmissionGgxEnergy: TransmissionGgxEnergy, wi: float3,
    state: PrimeFullCompactTransmissionState, samplingFlags: uint)
    ->float3
    where TransmissionGgxEnergy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    if (!primeFullOpenPbrMicrofacetEffectivelySmooth(state.specularMicrofacet) &&
        state.specularFresnel.ior != 1.0)
    {
        if (!primeFullOpenPbrIsSpecular(samplingFlags))
        {
            return float3(0.0);
        }
    }
    else if (!primeFullOpenPbrIsDelta(samplingFlags))
    {
        return float3(0.0);
    }
    let helperR : float = state.geometryThinWalled != 0u
                              ? primeFullOpenPbrMicrofacetDielectricMsCompensation(
                                    transmissionGgxEnergy, state.specularMicrofacet, abs(wi.z),
                                    state.specularFresnel.ior)
                                    .z
                              : state.transmissionMultipleScattering.z;
    let reflected : float = primeFullOpenPbrIsReflective(samplingFlags) ? helperR : 0.0;
    let transmitted : float = primeFullOpenPbrIsTransmissive(samplingFlags) ? 1.0 - helperR : 0.0;
    return reflected * state.specularFresnel.color + transmitted * state.transmissionTint;
}
