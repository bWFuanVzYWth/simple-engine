// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/pt.slang";

import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/service/support/conversion.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/opaque/state.slang";

import "bsdf/openpbr/opaque/traits.slang";
import "bsdf/openpbr/opaque/evaluate.slang";
import "bsdf/openpbr/opaque/sample.slang";
import "bsdf/openpbr/dielectric/evaluate.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/dielectric/state.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/pt_subsurface.slang";

// The legacy exact OpenPBR supported-domain kernels remain unchanged. PT supplies
// only their source-reachable parameters, after a visibility query; no generic
// material/default graph or closure survives a query.
public func primeFullPtOpaqueState<Energy>(energy: Energy, baseColor: float3, roughness: float,
                                           interfaceIor: float, subsurfaceWeight: float,
                                           conductor: bool, f0: float3, f82: float3, wi: float3)
    ->PrimeFullCompactOpaqueState
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    return primeFullPtOpaqueStateImpl<false>(energy, baseColor, roughness, interfaceIor,
                                             subsurfaceWeight, conductor, f0, f82, wi);
}

public func primeFullPtOpaqueStateImpl<let Continuous : bool, let Smooth : bool = false, Energy>(
    energy: Energy, baseColor: float3, roughness: float, interfaceIor: float,
    subsurfaceWeight: float, conductor: bool, f0: float3, f82: float3, wi: float3)
    ->PrimeFullCompactOpaqueState
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var subsurfaceWeight = subsurfaceWeight;
    var state : PrimeFullCompactOpaqueState = {};
    state.metalness = conductor ? 1.0 : 0.0;
    state.kind = conductor                 ? PRIME_FULL_COMPACT_OPAQUE_CONDUCTOR
                 : subsurfaceWeight == 1.0 ? PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                 : subsurfaceWeight > 0.0  ? PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                                           : PRIME_FULL_COMPACT_OPAQUE_DIELECTRIC;
    state.microfacet.alpha = Smooth ? float2(0.0) : float2(roughness * roughness);
    if (conductor)
    {
        state.conductorFresnel.f0 = f0;
        state.conductorFresnel.f82 = f82;
        state.conductorFresnel.f90 = float3(1.0);
        state.conductorFresnel.weight = 1.0;
        state.conductorFresnel.energyF0 = clamp(f0, float3(0.0), float3(1.0));
        state.conductorEss =
            (!Continuous && primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet))
                ? 1.0
                : primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(state.microfacet, wi.z,
                                                                        float3(1.0), float3(1.0))
                      .x;
        return state;
    }
    state.baseColor = baseColor;
    state.subsurfaceWeight = subsurfaceWeight;
    state.subsurfaceColor = baseColor;
    if (subsurfaceWeight > 0.0)
    {
        state.subsurfaceColor = primeFullPtSubsurfaceColor(baseColor);
    }
    state.subsurfaceAnisotropy = PRIME_FULL_PT_SUBSURFACE_ANISOTROPY;
    state.dielectricFresnel.ior = interfaceIor;
    state.dielectricFresnel.energyIor = primeFullOpenPbrIorAdjustment(interfaceIor, 1.0);
    state.dielectricFresnel.color = float3(1.0);
    state.dielectricMultipleScattering = primeFullOpenPbrMicrofacetDielectricMsCompensation<Smooth>(
        energy, state.microfacet, wi.z, state.dielectricFresnel.energyIor);
    let diffuseEnergy : float3 = state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE &&
                                         subsurfaceWeight < 1.0
                                     ? baseColor
                                     : float3(0.0);
    let subsurfaceEnergy
        : float3 = subsurfaceWeight > 0.0
                       ? primeFullCompactSubsurfaceEnergy(state, PRIME_FULL_OPENPBR_FLAG_ALL)
                       : float3(0.0);
    state.diffuseMix =
        primeFullCompactMakeMixState(subsurfaceWeight, diffuseEnergy, subsurfaceEnergy);
    let baseEnergy
        : float3 = state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                       ? primeFullCompactMixedDiffuseEnergy(state, PRIME_FULL_OPENPBR_FLAG_ALL)
                   : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                       ? subsurfaceEnergy
                       : primeFullCompactLambertEnergy(wi, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    state.layer = primeFullCompactMakeLayerState(
        baseEnergy, float3(1.0 - state.dielectricMultipleScattering.z),
        primeFullCompactDielectricEnergyImpl<Continuous>(wi, state, PRIME_FULL_OPENPBR_FLAG_ALL));
    return state;
}

// Canonical metalness mixes the two existing physical endpoints. The component
// proposal uses the same m as its response weight, without another directional-energy
// lookup or a second state. State is prepared only at its final local consumer.
public func primeFullPtMetallicState<Energy>(energy: Energy, color: float3, roughness: float,
                                             interfaceIor: float, subsurface: float,
                                             metalness: float, wi: float3)
    ->PrimeFullCompactOpaqueState
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var metalness = metalness;
    var state : PrimeFullCompactOpaqueState =
                    primeFullPtOpaqueState(energy, color, roughness, interfaceIor, subsurface,
                                           metalness == 1.0, saturate(color), float3(1.0), wi);
    if (metalness == 0.0 || metalness == 1.0)
        return state;
    state.metalness = metalness;
    state.conductorFresnel.f0 = saturate(color);
    state.conductorFresnel.f82 = float3(1.0);
    state.conductorFresnel.f90 = float3(1.0);
    state.conductorFresnel.weight = 1.0;
    state.conductorFresnel.energyF0 = saturate(color);
    state.conductorEss = primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet)
                             ? 1.0
                             : primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
                                   state.microfacet, wi.z, float3(1.0), float3(1.0))
                                   .x;
    return state;
}

public func primeFullPtTransmissionState<Energy>(energy: Energy, roughness: float,
                                                 interfaceIor: float, thinWalled: bool,
                                                 thinExtinction: float3, wi: float3)
    ->PrimeFullCompactTransmissionState
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var state : PrimeFullCompactTransmissionState = {};
    state.geometryThinWalled = thinWalled ? 1u : 0u;
    state.entering = wi.z >= 0.0 ? 1u : 0u;
    state.specularMicrofacet.alpha = float2(roughness * roughness);
    state.specularFresnel.ior = interfaceIor;
    state.specularFresnel.energyIor = primeFullOpenPbrIorAdjustment(interfaceIor, 1.0);
    state.specularFresnel.color = float3(1.0);
    state.transmissionTint = float3(1.0);
    state.transmissionMultipleScattering = float3(1.0, 1.0, 0.0);
    if (thinWalled)
    {
        state.transmissionVolume.extinction = thinExtinction;
        state.transmissionMicrofacet.alpha =
            clamp(state.specularMicrofacet.alpha *
                      primeFullOpenPbrThinDielectricRoughnessScaler2(interfaceIor),
                  float2(0.0), float2(1.0));
    }
    else
    {
        state.transmissionMultipleScattering = primeFullOpenPbrMicrofacetDielectricMsCompensation(
            energy, state.specularMicrofacet, wi.z, interfaceIor);
    }
    return state;
}

// MIS consumes the full closure response and its marginal proposal density,
// retaining the legacy sampler's exact selected direction and random mapping.
public func primeFullPtOpaqueSample(wi: float3, randomValue: float3,
                                    state: PrimeFullCompactOpaqueState)
    ->PrimeFullOpenPbrSample
{
    var sampled : PrimeFullOpenPbrSample = primeFullCompactOpaqueSample(wi, randomValue, state);
    if (state.metalness == 0.0 && primeFullOpenPbrHasSample(sampled) &&
        !primeFullOpenPbrIsDelta(sampled.throughput.flags))
    {
        let evaluated : PrimeFullCompactOpaqueEvaluation =
                            primeFullCompactOpaqueEvaluate(wi, sampled.wo, state);
        sampled.throughput.value = evaluated.throughput.value;
        sampled.pdf = evaluated.pdf;
    }
    return sampled;
}
