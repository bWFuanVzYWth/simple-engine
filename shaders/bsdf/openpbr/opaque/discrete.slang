// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/discrete.slang";

import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/opaque/state.slang";

public struct PrimeFullCompactOpaqueDiscreteState
{
    public var onb : PrimeFullOpenPbrOnb;
    public var microfacet : PrimeFullOpenPbrMicrofacet;
    public var conductorFresnel : PrimeFullOpenPbrConductorFresnel;
};

public func primeFullCompactOpaqueDiscreteStateInit(material: PrimeFullOpenPbrMaterial)
    ->PrimeFullCompactOpaqueDiscreteState
{
    var state : PrimeFullCompactOpaqueDiscreteState;
    state.onb = material.geometry.onb;
    state.microfacet.alpha = primeFullOpenPbrSpecularNdfRoughnesses(
        material.specular.roughness, material.specular.roughnessAnisotropy);
    state.conductorFresnel.f0 = material.base.color * material.weight.base;
    state.conductorFresnel.f82 = material.specular.color;
    state.conductorFresnel.f90 = float3(1.0);
    state.conductorFresnel.weight = material.weight.specular;
    state.conductorFresnel.energyF0 =
        clamp(material.base.color * material.weight.base * material.weight.specular, float3(0.0),
              float3(1.0));
    return state;
}

public func primeFullCompactOpaqueDiscreteStateFromFull(state: PrimeFullCompactOpaqueState)
    ->PrimeFullCompactOpaqueDiscreteState
{
    var result : PrimeFullCompactOpaqueDiscreteState;
    result.onb = state.onb;
    result.microfacet = state.microfacet;
    result.conductorFresnel = state.conductorFresnel;
    return result;
}

public func primeFullCompactConductorDiscreteSample(wi: float3,
                                                    state: PrimeFullCompactOpaqueDiscreteState,
                                                    samplingFlags: uint)
    ->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
    if (wi.z <= 0.0 || (samplingFlags & PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION) == 0u)
    {
        return result;
    }
    result.wo = reflect(-wi, float3(0.0, 0.0, 1.0));
    result.throughput.value =
        primeFullOpenPbrConductorFresnelUnpolarized(state.conductorFresnel, wi.z);
    result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
    result.pdf = 1.0;
    return result;
}

public func primeFullCompactConductorDiscreteEnergy(wi: float3,
                                                    state: PrimeFullCompactOpaqueDiscreteState)
    ->float3
{
    if (wi.z < 0.0)
    {
        return float3(0.0);
    }
    return primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
        state.microfacet, wi.z, state.conductorFresnel.energyF0,
        clamp(float3(state.conductorFresnel.weight), float3(0.0), float3(1.0)));
}
