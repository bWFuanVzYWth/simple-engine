// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 当前 OpenPBR 子集的控制流能力；Flow 为资源世代发布时证明的编译期上界。
/// 位描述可达分量和微面测度，不描述材质名称；diffuse／SSS 自身始终连续。
module flow_reference;
__exported import bsdf.openpbr.selection;

__exported import bsdf.openpbr.api;
__exported import bsdf.openpbr.specialized;
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";
import "bsdf/openpbr/math/fresnel.slang";
import "bsdf/openpbr/math/spectrum.slang";
import "bsdf/openpbr/opaque/discrete.slang";

import "bsdf/openpbr/pt.slang";
import "bsdf/openpbr/pt_subsurface.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/dielectric/thin_wall.slang";
import "bsdf/openpbr/dielectric/refract.slang";
import "bsdf/openpbr/dielectric/contract.slang";
import "bsdf/openpbr/service/support/sanitization.slang";
import "bsdf/openpbr/service/support/conversion.slang";

namespace openpbr
{
public func flow_has<let Flow : uint, let Bits : uint>()->bool
{
    return (Flow & Bits) != 0u;
}

/// 整个闭包包含连续测度；光滑塑料的 diffuse 与 delta specular 可以同时存在。
public func flow_continuous<let Flow : uint>()->bool
{
    return flow_has<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                              FLOW_CONTINUOUS_TRANSMISSION>();
}

/// 与is_discrete_only相同的全闭包分类；Flow由资源世代证明，纯测度域不保留数值分类分支。
/// 混合金属的非金属null proposal仍保留原metalness判定，不能把它当成纯金属。
public func is_discrete_only_flow<let Flow : uint>(material: Material, incident: float4,
                                                   transmitted: float4)
    ->bool
{
    static const let Delta : bool = (Flow & (FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)) != 0;
    static const let Transmission
        : bool = (Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) != 0;
    static const let Conductor : bool = (Flow & FLOW_CONDUCTOR_REFLECTION) != 0;
    if (!Delta || !(Transmission || Conductor))
        return false;
    if (!flow_continuous<Flow>() && (Transmission || (Flow & FLOW_METAL_MIX) == 0))
        return true;
    return is_discrete_only(material, incident, transmitted);
}

internal func flow_transmission<let Flow : uint>()->bool
{
    return flow_has<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>();
}

internal struct LegacyFlow<let Flow : uint>
{
    static const let capability : uint =
        FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has
            ? (FlowProperties<Flow>.continuous ? CAP_SOLID_DIELECTRIC : CAP_DELTA_SOLID_DIELECTRIC)
        : FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has
            ? (FlowProperties<Flow>.continuous ? CAP_THIN_DIELECTRIC : CAP_DELTA_THIN_DIELECTRIC)
        : !FlowProperties<Flow>.layer && !FlowMask<Flow, FLOW_METAL_MIX>.has
            ? (FlowProperties<Flow>.continuous ? CAP_CONDUCTOR : CAP_DELTA_CONDUCTOR)
        : FlowMask<Flow, FLOW_SUBSURFACE>.has ? CAP_OPAQUE_SUBSURFACE
                                              : CAP_OPAQUE_DIELECTRIC;
};

internal func require_flow<let Flow : uint>()->void
{
    static_assert((Flow & ~4095u) == 0u, "unknown OpenPBR flow bits");
    static_assert(FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has ==
                      FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION>.has,
                  "reflection components require their measurement domains");
    static_assert(FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ==
                      FlowMask<Flow, FLOW_DELTA_TRANSMISSION | FLOW_CONTINUOUS_TRANSMISSION>.has,
                  "transmission topology requires its measurement domains");
    static_assert((Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) !=
                      (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION),
                  "solid and thin are distinct boundary contracts");
    static_assert(
        !FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
            !FlowMask<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONDUCTOR_REFLECTION>.has,
        "opaque and transmission are disjoint in the supported subset");
    static_assert(!FlowMask<Flow, FLOW_METAL_MIX>.has ||
                      (FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has &&
                       !FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has),
                  "metal mixture requires an opaque conductor");
    static_assert(!FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has || !FlowProperties<Flow>.layer ||
                      FlowMask<Flow, FLOW_METAL_MIX>.has,
                  "mixed metalness must retain its proposal control");
    static_assert(!FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
                      FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      (FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has &&
                       !FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION |
                                           FLOW_DELTA_REFLECTION>.has),
                  "matched transmission is delta only");
    static_assert(!FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has ||
                      ((!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has ||
                        FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<Flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "solid reflection/transmission domains disagree");
    static_assert(!FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has ||
                      !FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      ((FlowMask<Flow, FLOW_DELTA_REFLECTION>.has ==
                        FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<Flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "thin reflection/transmission domains disagree");
}

static const let binary_metal : bool = (openpbr_composition::SelectedVariant & 2048) != 0;
static const let binary_sss : bool = (openpbr_composition::SelectedVariant & 4096) != 0;

internal func flow_opaque<let Flow : uint, Provider>(energy: Provider, material: Material,
                                                     wi: float3, incident: float4)
    ->PrimeFullCompactOpaqueState
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    var color : float3 = material.base_roughness.xyz;
    var state : PrimeFullCompactOpaqueState = {};
    state.microfacet.alpha = FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has
                                 ? float2(material.base_roughness.w * material.base_roughness.w)
                                 : float2(0);
    if (FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has)
    {
        state.metalness = FlowMask<Flow, FLOW_METAL_MIX>.has ? material.surface.y : 1.0;
        if (!binary_metal || state.metalness > 0.0)
        {
            state.conductorFresnel.f0 = saturate(color);
            state.conductorFresnel.f82 = 1;
            state.conductorFresnel.f90 = 1;
            state.conductorFresnel.weight = 1;
            state.conductorFresnel.energyF0 = saturate(color);
            state.conductorEss =
                !FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has ||
                        (FlowMask<Flow, FLOW_DELTA_REFLECTION>.has &&
                         primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet))
                    ? 1.0
                    : primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(state.microfacet, wi.z,
                                                                            float3(1), float3(1))
                          .x;
        }
        if (!FlowProperties<Flow>.layer || (binary_metal && state.metalness == 1.0))
        {
            state.kind = PRIME_FULL_COMPACT_OPAQUE_CONDUCTOR;
            return state;
        }
    }
    var baseEnergy : float3 = 0;
    if (FlowProperties<Flow>.base)
    {
        state.baseColor = color;
        baseEnergy = wi.z >= 0 ? color : float3(0);
        if (FlowMask<Flow, FLOW_SUBSURFACE>.has)
        {
            let w : float = FlowMask<Flow, FLOW_DIFFUSE>.has ? material.surface.z : 1.0;
            state.subsurfaceWeight = w;
            state.subsurfaceAnisotropy = PRIME_FULL_PT_SUBSURFACE_ANISOTROPY;
            if (w > 0.0)
            {
                state.subsurfaceColor = primeFullPtSubsurfaceColor(color);
                let sss : float3 =
                              primeFullCompactSubsurfaceEnergy(state, PRIME_FULL_OPENPBR_FLAG_ALL);
                if (binary_sss || !FlowMask<Flow, FLOW_DIFFUSE>.has)
                {
                    state.kind = PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE;
                    baseEnergy = sss;
                }
                else
                {
                    state.kind = w == 1.0 ? PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                                          : PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE;
                    baseEnergy = color * (1.0 - w) + sss * w;
                    state.diffuseMix = primeFullCompactMakeMixState(w, color, sss);
                }
            }
            else
                state.diffuseMix.firstSampleWeight = 1;
        }
    }
    if (FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has)
    {
        state.dielectricFresnel.ior = material.surface.x / incident.x;
        state.dielectricFresnel.energyIor =
            primeFullOpenPbrIorAdjustment(state.dielectricFresnel.ior, 1.0);
        state.dielectricFresnel.color = 1;
        state.dielectricMultipleScattering = primeFullOpenPbrMicrofacetDielectricMsCompensation<
            !FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has>(energy, state.microfacet, wi.z,
                                                             state.dielectricFresnel.energyIor);
        if (FlowProperties<Flow>.base)
            state.layer = primeFullCompactMakeLayerState(
                baseEnergy, float3(1.0 - state.dielectricMultipleScattering.z),
                primeFullCompactDielectricEnergyImpl<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                    wi, state, PRIME_FULL_OPENPBR_FLAG_ALL));
        else
            state.layer.specularSampleWeight =
                state.dielectricMultipleScattering.z > 0.0 ? 1.0 : 0.0;
    }
    else
    {
        state.dielectricFresnel.ior = 1;
        state.dielectricFresnel.energyIor = 1;
        state.dielectricFresnel.color = 1;
        state.dielectricMultipleScattering = float3(1, 1, 0);
        state.layer.transmittanceIn = 1;
        state.layer.baseSampleWeight = any(color > 0) ? 1 : 0;
    }
    return state;
}

internal func flow_layer_evaluate<let Flow : uint>(wi: float3, wo: float3,
                                                   state: PrimeFullCompactOpaqueState,
                                                   reflection: PrimeFullOpenPbrReflectiveEvalBase)
    ->PrimeFullCompactOpaqueEvaluation
{
    if (!FlowProperties<Flow>.base)
    {
        var result : PrimeFullCompactOpaqueEvaluation = {};
        if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
        {
            result.throughput =
                primeFullCompactDielectricEvalFromBase<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                    reflection, state);
            if (state.layer.specularSampleWeight > 0.0)
                result.pdf = reflection.pdf;
        }
        return result;
    }
    if (FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has &&
        flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>() && !flow_has<Flow, FLOW_SUBSURFACE>())
        return primeFullCompactOpaqueLayerEvaluateFromBase<
            !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(wi, wo, state, reflection);
    var base : PrimeFullOpenPbrThroughput;
    var base_pdf : float;
    if (flow_has<Flow, FLOW_SUBSURFACE>() && flow_has<Flow, FLOW_DIFFUSE>())
    {
        if (binary_sss)
        {
            if (state.subsurfaceWeight == 1.0)
            {
                base = primeFullCompactSubsurfaceEval<true>(wi, wo, state,
                                                            PRIME_FULL_OPENPBR_FLAG_ALL);
                base_pdf =
                    primeFullCompactSubsurfacePdf<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
            }
            else
            {
                base = primeFullCompactLambertEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
                base_pdf = primeFullCompactLambertPdf(wi, wo);
            }
        }
        else
        {
            base =
                primeFullCompactMixedDiffuseEval<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
            base_pdf =
                primeFullCompactMixedDiffusePdf<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        }
    }
    else if (flow_has<Flow, FLOW_SUBSURFACE>())
    {
        base = primeFullCompactSubsurfaceEval<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        base_pdf = primeFullCompactSubsurfacePdf<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    }
    else
    {
        base = primeFullCompactLambertEval(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        base_pdf = primeFullCompactLambertPdf(wi, wo);
    }
    var result : PrimeFullCompactOpaqueEvaluation;
    if (!FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has)
    {
        result.throughput = base;
        result.pdf = state.layer.baseSampleWeight > 0.0 ? base_pdf : 0.0;
        return result;
    }
    var specular : PrimeFullOpenPbrThroughput = primeFullOpenPbrZeroThroughput();
    var specular_pdf = 0.0;
    if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has &&
        FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has)
    {
        specular =
            primeFullCompactDielectricEvalFromBase<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                reflection, state);
        specular_pdf = reflection.pdf;
    }
    result.throughput = primeFullCompactLayerEval(base, specular, state.layer);
    result.pdf = state.layer.baseSampleWeight + state.layer.specularSampleWeight > 0.0
                     ? lerp(base_pdf, specular_pdf, state.layer.specularSampleWeight)
                     : 0.0;
    return result;
}

internal func flow_opaque_evaluate<let Flow : uint>(wi: float3, wo: float3,
                                                    state: PrimeFullCompactOpaqueState)
    ->PrimeFullCompactOpaqueEvaluation
{
    if (!FlowProperties<Flow>.layer)
    {
        var result : PrimeFullCompactOpaqueEvaluation = specialized_opaque_evaluate<
                         CAP_CONDUCTOR, !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(wi, wo, state);
        if (FlowMask<Flow, FLOW_METAL_MIX>.has)
        {
            result.throughput.value *= state.metalness;
            result.pdf *= state.metalness;
        }
        return result;
    }
    var reflection : PrimeFullOpenPbrReflectiveEvalBase = {};
    if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
        reflection =
            primeFullOpenPbrReflectiveEvalBaseImpl<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                wi, wo, state.microfacet, PRIME_FULL_OPENPBR_FLAG_ALL,
                PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    var second : PrimeFullCompactOpaqueEvaluation = {};
    if (FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has && (!binary_metal || state.metalness > 0))
    {
        if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
        {
            second.throughput =
                primeFullCompactConductorEvalFromBase<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                    reflection, state);
            second.pdf = reflection.pdf;
        }
        if (binary_metal && state.metalness == 1)
            return second;
    }
    var first : PrimeFullCompactOpaqueEvaluation =
                    flow_layer_evaluate<Flow>(wi, wo, state, reflection);
    if (!FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has || binary_metal)
        return first;

    first.throughput =
        primeFullCompactMixEval(first.throughput, second.throughput, state.metalness);
    first.pdf = lerp(first.pdf, second.pdf, state.metalness);
    return first;
}

internal func flow_layer_sample<let Flow : uint>(wi: float3, u: float3,
                                                 state: PrimeFullCompactOpaqueState)
    ->PrimeFullOpenPbrSample
{
    if (!FlowProperties<Flow>.base)
    {
        if (!(state.layer.specularSampleWeight > 0.0))
            return primeFullOpenPbrZeroSample();
        if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
            return primeFullCompactDielectricSampleImpl<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                wi, u, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        let smooth : PrimeFullOpenPbrMicrofacet = {};
        let base
            : PrimeFullOpenPbrReflectiveSampleBase = primeFullOpenPbrReflectiveSampleBase(
                  wi, u, smooth, PRIME_FULL_OPENPBR_FLAG_ALL, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
        var result : PrimeFullOpenPbrSample = base.bsdfSample;
        result.throughput.value *=
            primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine);
        return result;
    }
    if (FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has &&
        flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>() && !flow_has<Flow, FLOW_SUBSURFACE>())
        return primeFullCompactOpaqueLayerSampleImpl<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
            wi, u, state, false);
    if (state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0)
        return primeFullOpenPbrZeroSample();
    if (FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has && u.z <= state.layer.specularSampleWeight &&
        state.layer.specularSampleWeight > 0.0)
    {
        if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
        {
            var result : PrimeFullOpenPbrSample = primeFullCompactDielectricSampleImpl<
                             !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                             wi, float3(u.xy, u.z / state.layer.specularSampleWeight), state,
                             PRIME_FULL_OPENPBR_FLAG_ALL);
            result.pdf *= state.layer.specularSampleWeight;
            return result;
        }
        let smooth : PrimeFullOpenPbrMicrofacet = {};
        let base
            : PrimeFullOpenPbrReflectiveSampleBase = primeFullOpenPbrReflectiveSampleBase(
                  wi, u, smooth, PRIME_FULL_OPENPBR_FLAG_ALL, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
        var result : PrimeFullOpenPbrSample = base.bsdfSample;
        result.throughput.value *=
            primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine);
        result.pdf *= state.layer.specularSampleWeight;
        return result;
    }
    let random : float3 = FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has
                              ? float3(u.xy, (u.z - state.layer.specularSampleWeight) /
                                                 state.layer.baseSampleWeight)
                              : u;
    var result : PrimeFullOpenPbrSample;
    if (flow_has<Flow, FLOW_SUBSURFACE>() && flow_has<Flow, FLOW_DIFFUSE>())
    {
        if (state.subsurfaceWeight == 0.0)
            result = primeFullCompactLambertSample(wi, random, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        else if (binary_sss || state.subsurfaceWeight == 1.0)
            result = primeFullCompactSubsurfaceSample<true>(wi, random, state,
                                                            PRIME_FULL_OPENPBR_FLAG_ALL);
        else
            result = primeFullCompactMixedDiffuseSample<true, true>(wi, random, state,
                                                                    PRIME_FULL_OPENPBR_FLAG_ALL);
    }
    else if (flow_has<Flow, FLOW_SUBSURFACE>())
        result =
            primeFullCompactSubsurfaceSample<true>(wi, random, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    else
        result = primeFullCompactLambertSample(wi, random, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    if (FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has)
    {
        result.throughput =
            primeFullOpenPbrThroughputScale(result.throughput, state.layer.transmittanceIn);
        result.pdf *= state.layer.baseSampleWeight;
    }
    return result;
}

internal func flow_opaque_sample<let Flow : uint>(wi: float3, u: float3,
                                                  state: PrimeFullCompactOpaqueState)
    ->PrimeFullOpenPbrSample
{
    if (!FlowProperties<Flow>.layer)
    {
        if (FlowMask<Flow, FLOW_METAL_MIX>.has && u.z >= state.metalness)
            return primeFullOpenPbrZeroSample();
        var result : PrimeFullOpenPbrSample;
        if (!FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
            result = primeFullCompactConductorDiscreteSample(
                wi, primeFullCompactOpaqueDiscreteStateFromFull(state),
                PRIME_FULL_OPENPBR_FLAG_ALL);
        else
            result = specialized_opaque_sample<CAP_CONDUCTOR,
                                               !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                wi, FlowMask<Flow, FLOW_METAL_MIX>.has ? float3(u.xy, u.z / state.metalness) : u,
                state);
        if (FlowMask<Flow, FLOW_METAL_MIX>.has)
        {
            if (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has && state.metalness < 1.0 &&
                primeFullOpenPbrHasSample(result) &&
                (!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has ||
                 primeFullOpenPbrIsNonDelta(result.throughput.flags)))
            {
                let evaluated
                    : PrimeFullCompactOpaqueEvaluation =
                          specialized_opaque_evaluate<CAP_CONDUCTOR,
                                                      !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                              wi, result.wo, state);
                result.throughput = evaluated.throughput;
                result.pdf = evaluated.pdf;
                // 原 marginal 保留零非金属层的 diffuse 标签，不需要恢复其求值。
                result.throughput.flags |= PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION;
            }
            result.throughput.value *= state.metalness;
            result.pdf *= state.metalness;
        }
        return result;
    }
    var result : PrimeFullOpenPbrSample;
    if (binary_metal && FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has && state.metalness == 1.0)
    {
        if (!FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has)
            return primeFullCompactConductorDiscreteSample(
                wi, primeFullCompactOpaqueDiscreteStateFromFull(state),
                PRIME_FULL_OPENPBR_FLAG_ALL);
        return primeFullCompactConductorSampleImpl<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
            wi, u, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    }
    if (!binary_metal && flow_has<Flow, FLOW_CONDUCTOR_REFLECTION>())
    {
        let m : float = state.metalness;
        if (u.z < m)
        {
            if (!flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>())
                result = primeFullCompactConductorDiscreteSample(
                    wi, primeFullCompactOpaqueDiscreteStateFromFull(state),
                    PRIME_FULL_OPENPBR_FLAG_ALL);
            else
                result =
                    primeFullCompactConductorSampleImpl<!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
                        wi, float3(u.xy, u.z / m), state, PRIME_FULL_OPENPBR_FLAG_ALL);
        }
        else
            result = flow_layer_sample<Flow>(wi, float3(u.xy, (u.z - m) / (1.0 - m)), state);
        if (!primeFullOpenPbrHasSample(result))
            return result;
        if (m == 1.0)
            return result;
        if (flow_has<Flow, FLOW_DELTA_REFLECTION>() &&
            primeFullOpenPbrIsDelta(result.throughput.flags))
        {
            if (m == 0.0)
                return result;
            result.throughput.value =
                lerp(primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, wi.z),
                     primeFullOpenPbrConductorFresnelUnpolarized(state.conductorFresnel, wi.z), m);
            result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
            result.pdf = m + (1.0 - m) * state.layer.specularSampleWeight;
            return result;
        }
    }
    else
    {
        result = flow_layer_sample<Flow>(wi, u, state);
        if (!primeFullOpenPbrHasSample(result) ||
            (flow_has<Flow, FLOW_DELTA_REFLECTION>() &&
             primeFullOpenPbrIsDelta(result.throughput.flags)))
            return result;
    }
    let evaluated : PrimeFullCompactOpaqueEvaluation =
                        flow_opaque_evaluate<Flow>(wi, result.wo, state);
    if (!binary_metal && flow_has<Flow, FLOW_CONDUCTOR_REFLECTION>() && state.metalness > 0.0 &&
        state.metalness < 1.0)
        result.throughput = evaluated.throughput;
    else
        result.throughput.value = evaluated.throughput.value;
    result.pdf = evaluated.pdf;
    return result;
}

/// 编译期能力的连续支持域；不读取纹理。动态 TIR、有效方向等仍由实际闭包处理。
public func may_evaluate_flow<let Flow : uint>(material: Material, normal: float3, view: float3,
                                               direction: float3, incident: float4,
                                               transmitted: float4)
    ->bool
{
    require_flow<Flow>();
    if (!flow_continuous<Flow>())
        return false;
    if (flow_transmission<Flow>())
        return true;
    if (FlowMask<Flow, FLOW_SUBSURFACE>.has)
    {
        let dielectric_base : bool =
                                  !FlowMask<Flow, FLOW_METAL_MIX>.has || material.surface.y < 1.0;
        let subsurface : bool = !FlowMask<Flow, FLOW_DIFFUSE>.has || material.surface.z > 0.0;
        if (dielectric_base && subsurface)
            return true;
    }
    if (!FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has && FlowMask<Flow, FLOW_METAL_MIX>.has &&
        material.surface.y == 1.0)
        return false;
    return may_evaluate_specialized<CAP_OPAQUE_DIELECTRIC>(material, normal, view, direction,
                                                           incident, transmitted);
}

/// 与未裁剪 evaluate 相同的完整连续响应与 marginal PDF；Flow 必须覆盖全部可达分量。
public func evaluate_flow<let Flow : uint, Provider>(energy: Provider, material: Material,
                                                     normal: float3, view: float3,
                                                     direction: float3, incident: float4,
                                                     transmitted: float4)
    ->Evaluation
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_flow<Flow>();
    if (!flow_continuous<Flow>())
        return { float3(0.0), 0.0 };
    if (flow_transmission<Flow>())
    {
        if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has &&
            !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has &&
            FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has)
        {
            let ior : float = incident.x > transmitted.x ? incident.x / transmitted.x
                                                         : transmitted.x / incident.x;
            if (ior == 1.0)
                return { float3(0.0), 0.0 };
            return evaluate_solid_continuous(energy, material, normal, view, direction, incident,
                                             transmitted);
        }
        if (FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has &&
            !FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has)
        {
            let onb
                : PrimeFullOpenPbrOnb =
                      specialized_frame<CAP_THIN_DIELECTRIC>(normal, view, incident, transmitted);
            let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
            let wo : float3 = primeFullOpenPbrOnbToLocal(onb, direction);
            let state : PrimeFullCompactTransmissionState = primeFullPtTransmissionState(
                            energy, material.base_roughness.w, transmitted.x / incident.x, true,
                            transmitted.yzw * material.surface.w, wi);
            let evaluated : PrimeFullCompactTransmissionEvaluation =
                                primeFullCompactTransmissionEvaluateRaw<true>(
                                    wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
            return specialized_evaluation(evaluated.throughput, evaluated.pdf);
        }
        if (flow_has<Flow, FLOW_SOLID_TRANSMISSION>() &&
            !flow_has<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>())
            return evaluate_solid_continuous(energy, material, normal, view, direction, incident,
                                             transmitted);
        return evaluate_specialized<LegacyFlow<Flow>.capability>(energy, material, normal, view,
                                                                 direction, incident, transmitted);
    }
    let onb : PrimeFullOpenPbrOnb =
                  specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
    let wo : float3 = primeFullOpenPbrOnbToLocal(onb, direction);
    if (!flow_has<Flow, FLOW_SUBSURFACE>() && wo.z < 0.0)
        return { float3(0.0), 0.0 };
    let evaluated
        : PrimeFullCompactOpaqueEvaluation =
              flow_opaque_evaluate<Flow>(wi, wo, flow_opaque<Flow>(energy, material, wi, incident));
    return specialized_evaluation < FlowMask<Flow, FLOW_SUBSURFACE>.has &&
           !FlowMask<Flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has >
               (evaluated.throughput, evaluated.pdf);
}

internal func flow_matched_solid(normal: float3, view: float3, incident: float4,
                                 transmitted: float4)
    ->Sample
{
    let onb
        : PrimeFullOpenPbrOnb =
              specialized_frame<CAP_DELTA_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
    let refracted : PrimeFullOpenPbrRefractResult =
                        primeFullOpenPbrDielectricRefract(1.0, -wi, float3(0.0, 0.0, 1.0));
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (refracted.valid != 0u && wi.z * refracted.wo.z < 0.0)
    {
        var direction : float3 = primeFullOpenPbrOnbToWorld(onb, refracted.wo);
        if (primeBsdfDirection(direction))
        {
            result.direction = direction;
            result.response = float3(1.0);
            result.events = TRANSMISSION | DISCRETE;
            result.pdf = 1.0;
            result.medium = transmitted;
        }
    }
    return result;
}

/// 与未裁剪 sample 相同的随机映射、完整 response/PDF、事件、eta 与介质转移。
/// 只消除能力排除的计算；delta 与连续分量共存时维持两种不同测度。
public func sample_flow<let Flow : uint, Provider>(energy: Provider, material: Material,
                                                   normal: float3, view: float3, u: float3,
                                                   incident: float4, transmitted: float4)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_flow<Flow>();
    if (!FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION | FLOW_DIFFUSE |
                            FLOW_SUBSURFACE | FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has)
    {
        var result : Sample = {};
        result.relative_eta = 1.0;
        result.medium = incident;
        return result;
    }
    if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has &&
        !FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has)
        return flow_matched_solid(normal, view, incident, transmitted);
    if (FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has &&
        !FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has)
    {
        var result : Sample = {};
        result.relative_eta = 1.0;
        result.medium = incident;
        let cosine : float = abs(dot(normal, view));
        var response : float3 = cosine > 0.0 ? exp(-transmitted.yzw * material.surface.w / cosine)
                                             : float3(0.0);
        if (cosine > 0.0 && primeFullOpenPbrSpectrumToWeight(response) > 0.0)
        {
            result.direction = -view;
            result.response = response;
            result.pdf = 1.0;
            result.events = TRANSMISSION | DISCRETE;
        }
        return result;
    }
    if (!flow_continuous<Flow>() &&
        ((!FlowProperties<Flow>.layer && !FlowMask<Flow, FLOW_METAL_MIX>.has) ||
         flow_transmission<Flow>()))
        return sample_specialized<LegacyFlow<Flow>.capability>(energy, material, normal, view, u,
                                                               incident, transmitted);
    if (!FlowProperties<Flow>.base &&
        !FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION | FLOW_SOLID_TRANSMISSION |
                            FLOW_THIN_TRANSMISSION | FLOW_CONTINUOUS_REFLECTION>.has)
    {
        let onb : PrimeFullOpenPbrOnb =
                      specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
        let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
        var result : PrimeFullOpenPbrSample = primeFullOpenPbrZeroSample();
        var fresnel : PrimeFullOpenPbrSpecularFresnel;
        fresnel.ior = material.surface.x / incident.x;
        fresnel.energyIor = primeFullOpenPbrIorAdjustment(fresnel.ior, 1.0);
        fresnel.color = float3(1.0);
        if (wi.z > 0.0 && fresnel.ior != 1.0)
        {
            result.wo = reflect(-wi, float3(0.0, 0.0, 1.0));
            result.throughput.value = primeFullOpenPbrSpecularFresnelUnpolarized(fresnel, wi.z);
            result.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
            result.pdf = 1.0;
        }
        return specialized_result<CAP_OPAQUE_DIELECTRIC, false, true, REFLECTION | DISCRETE>(
            result, onb, view, incident, transmitted);
    }
    if (flow_transmission<Flow>())
    {
        if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has &&
            !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has &&
            FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has)
        {
            let ior : float = incident.x > transmitted.x ? incident.x / transmitted.x
                                                         : transmitted.x / incident.x;
            if (ior != 1.0)
                return sample_solid_continuous(energy, material, normal, view, u, incident,
                                               transmitted);
            return flow_matched_solid(normal, view, incident, transmitted);
        }
        if (FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has &&
            !FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has)
        {
            let onb
                : PrimeFullOpenPbrOnb =
                      specialized_frame<CAP_THIN_DIELECTRIC>(normal, view, incident, transmitted);
            let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
            let state : PrimeFullCompactTransmissionState = primeFullPtTransmissionState(
                            energy, material.base_roughness.w, transmitted.x / incident.x, true,
                            transmitted.yzw * material.surface.w, wi);
            return specialized_result<CAP_THIN_DIELECTRIC, true>(
                primeFullCompactTransmissionSampleRaw<true>(wi, u, state,
                                                            PRIME_FULL_OPENPBR_FLAG_ALL),
                onb, view, incident, transmitted);
        }
        if (flow_has<Flow, FLOW_SOLID_TRANSMISSION>() &&
            !flow_has<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>())
            return sample_solid_continuous(energy, material, normal, view, u, incident,
                                           transmitted);
        return sample_specialized<LegacyFlow<Flow>.capability>(energy, material, normal, view, u,
                                                               incident, transmitted);
    }
    let onb : PrimeFullOpenPbrOnb =
                  specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
    let sampled
        : PrimeFullOpenPbrSample =
              flow_opaque_sample<Flow>(wi, u, flow_opaque<Flow>(energy, material, wi, incident));
    return specialized_result<LegacyFlow<Flow>.capability,
                              !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(sampled, onb, view,
                                                                          incident, transmitted);
}

internal func flow_transmission_evaluate_prepared<let Flow : uint>(
    wi: float3, wo: float3, state: PrimeFullCompactTransmissionState)
    ->Evaluation
{
    var result : PrimeFullCompactTransmissionEvaluation;
    if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has && !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has)
        result = primeFullCompactTransmissionEvaluateRawSolidContinuous(
            wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    else
        result = primeFullCompactTransmissionEvaluateRaw<
            !FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has>(
            wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    return specialized_evaluation(result.throughput, result.pdf);
}

internal func flow_transmission_sample_prepared<let Flow : uint>(
    wi: float3, u: float3, state: PrimeFullCompactTransmissionState)
    ->PrimeFullOpenPbrSample
{
    if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has && !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has)
        return primeFullCompactTransmissionSampleRawSolidContinuous(wi, u, state,
                                                                    PRIME_FULL_OPENPBR_FLAG_ALL);
    return primeFullCompactTransmissionSampleRaw<
        !FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has>(
        wi, u, state, PRIME_FULL_OPENPBR_FLAG_ALL);
}

/// 按方向即时消费连续评估；direction=0/1 对应 first/second，只回调掩码请求的方向。
/// 回调在共享闭包状态有效期内执行，不得发起 callable 或主查询。
public interface IFlowEvaluationConsumer
{
    [mutating]
    func consume(direction: uint, evaluated: Evaluation)->void;
};

/// 正向组合：共享一次局部坐标和能量准备，仅求所请求的方向和散射。
public func shade_flow<let Flow : uint, Provider, let Directions : uint = 3>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4)
    ->Batch
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_flow<Flow>();
    static_assert((Directions & ~3u) == 0u, "unknown evaluation direction mask");
    var batch : Batch = {};
    var result : Sample = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (!flow_continuous<Flow>())
    {
        if (continuation)
            result = sample_flow<Flow>(energy, material, normal, view, u, incident, transmitted);
        return { batch.first, batch.second, result };
    }
    if ((evaluation_mask & Directions) == 0u && !continuation)
        return { batch.first, batch.second, result };
    if (flow_transmission<Flow>())
    {
        let ior : float = FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has ? transmitted.x / incident.x
                          : incident.x > transmitted.x               ? incident.x / transmitted.x
                                                                     : transmitted.x / incident.x;
        if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has &&
            !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has &&
            FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has && ior == 1.0)
        {
            if (continuation)
                result = flow_matched_solid(normal, view, incident, transmitted);
            return { batch.first, batch.second, result };
        }
        let onb : PrimeFullOpenPbrOnb = specialized_frame<LegacyFlow<Flow>.capability>(
                      normal, view, incident, transmitted);
        let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
        var state : PrimeFullCompactTransmissionState =
                        primeFullPtTransmissionState(energy, material.base_roughness.w, ior,
                                                     FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has,
                                                     transmitted.yzw * material.surface.w, wi);
        if (FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has)
            state.entering = 1u;
        if ((Directions & 1u) != 0u && (evaluation_mask & 1u) != 0u)
            batch.first = flow_transmission_evaluate_prepared<Flow>(
                wi, primeFullOpenPbrOnbToLocal(onb, first), state);
        if ((Directions & 2u) != 0u && (evaluation_mask & 2u) != 0u)
            batch.second = flow_transmission_evaluate_prepared<Flow>(
                wi, primeFullOpenPbrOnbToLocal(onb, second), state);
        if (continuation)
            result = specialized_result<
                LegacyFlow<Flow>.capability,
                !FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>.has>(
                flow_transmission_sample_prepared<Flow>(wi, u, state), onb, view, incident,
                transmitted);
        return { batch.first, batch.second, result };
    }
    let onb : PrimeFullOpenPbrOnb =
                  specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    let wi : float3 = primeFullOpenPbrOnbToLocal(onb, view);
    let state : PrimeFullCompactOpaqueState = flow_opaque<Flow>(energy, material, wi, incident);
    if ((Directions & 1u) != 0u && (evaluation_mask & 1u) != 0u)
    {
        let evaluated
            : PrimeFullCompactOpaqueEvaluation =
                  flow_opaque_evaluate<Flow>(wi, primeFullOpenPbrOnbToLocal(onb, first), state);
        batch.first =
            specialized_evaluation < FlowMask<Flow, FLOW_SUBSURFACE>.has &&
            !FlowMask<Flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has >
                (evaluated.throughput, evaluated.pdf);
    }
    if ((Directions & 2u) != 0u && (evaluation_mask & 2u) != 0u)
    {
        let evaluated
            : PrimeFullCompactOpaqueEvaluation =
                  flow_opaque_evaluate<Flow>(wi, primeFullOpenPbrOnbToLocal(onb, second), state);
        batch.second =
            specialized_evaluation < FlowMask<Flow, FLOW_SUBSURFACE>.has &&
            !FlowMask<Flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has >
                (evaluated.throughput, evaluated.pdf);
    }
    if (continuation)
        result = specialized_result<LegacyFlow<Flow>.capability,
                                    !FlowMask<Flow, FLOW_DELTA_REFLECTION>.has>(
            flow_opaque_sample<Flow>(wi, u, state), onb, view, incident, transmitted);
    return { batch.first, batch.second, result };
}

/// 延迟着色适配器按方向接收上述数值结果。
public func shade_flow_consume<let Flow : uint, Provider, Consumer, let Directions : uint = 3>(
    energy: Provider, material: Material, normal: float3, view: float3, first: float3,
    second: float3, evaluation_mask: uint, u: float3, continuation: bool, incident: float4,
    transmitted: float4, inout consumer: Consumer)
    ->Sample
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    let b : Batch = shade_flow<Flow, Provider, Directions>(energy, material, normal, view, first,
                                                           second, evaluation_mask, u, continuation,
                                                           incident, transmitted);
    if ((Directions & evaluation_mask & 1u) != 0)
        consumer.consume(0u, b.first);
    if ((Directions & evaluation_mask & 2u) != 0)
        consumer.consume(1u, b.second);
    return b.scattered;
}

}
