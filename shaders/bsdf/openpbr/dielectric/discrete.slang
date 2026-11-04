// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/discrete.slang";

import "bsdf/openpbr/contract/volume_sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/volume.slang";
import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/dielectric/thin_wall.slang";
import "bsdf/openpbr/dielectric/volume.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/delta.slang";
import "bsdf/openpbr/math/spectrum.slang";

public PrimeFullOpenPbrSample primeFullCompactRefractiveDiscreteSample(
    float3 wi, float randomEvent, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    return primeFullRefractiveDeltaSample(wi, randomEvent, state.specularFresnel, samplingFlags);
}

public PrimeFullOpenPbrSample primeFullCompactThinWallDiscreteSampleRaw(
    float3 wi, float randomEvent, PrimeFullCompactTransmissionState state, uint samplingFlags)
{
    return primeFullThinWallDeltaSample(wi, randomEvent, state.specularFresnel,
                                        state.transmissionTint, state.transmissionVolume.extinction,
                                        samplingFlags);
}

public PrimeFullOpenPbrSampleResult primeFullCompactTransmissionDiscreteSample(
    float3 wi, float randomEvent, PrimeFullCompactTransmissionState state, uint samplingFlags,
    PrimeFullOpenPbrVolume medium)
{
    PrimeFullCompactTransmissionState closureState = state;
    if (closureState.geometryThinWalled == 0u)
    {
        closureState.entering = 1u;
    }
    PrimeFullOpenPbrSampleResult result;
    result.medium = medium;
    result.rayT = state.rayT;
    if (closureState.geometryThinWalled != 0u)
    {
        result.bsdfSample =
            primeFullCompactThinWallDiscreteSampleRaw(wi, randomEvent, closureState, samplingFlags);
    }
    else
    {
        result.bsdfSample = primeFullCompactRefractiveDiscreteSample(
            primeFullCompactTransmissionMaterialFrame(wi, closureState), randomEvent, closureState,
            samplingFlags);
        if (result.bsdfSample.throughput.flags != PRIME_FULL_OPENPBR_FLAG_NONE)
        {
            result.bsdfSample.wo =
                primeFullCompactTransmissionMaterialFrame(result.bsdfSample.wo, closureState);
            if (primeFullOpenPbrIsTransmissive(result.bsdfSample.throughput.flags))
            {
                result.bsdfSample.throughput.value *= closureState.transmissionTint;
            }
        }
    }
    primeFullCompactApplyTransmissionVolumeTransition(state, result);
    return result;
}
