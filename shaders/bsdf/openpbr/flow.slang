// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 当前 OpenPBR 子集的控制流能力；Flow 为资源世代发布时证明的编译期上界。
/// 位描述可达分量和微面测度，不描述材质名称；diffuse／SSS 自身始终连续。
module flow;

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
public static const uint FLOW_DIFFUSE = 1u << 0;
public static const uint FLOW_DIELECTRIC_REFLECTION = 1u << 1;
public static const uint FLOW_CONDUCTOR_REFLECTION = 1u << 2;
public static const uint FLOW_SUBSURFACE = 1u << 3;
public static const uint FLOW_SOLID_TRANSMISSION = 1u << 4;
public static const uint FLOW_THIN_TRANSMISSION = 1u << 5;
public static const uint FLOW_DELTA_REFLECTION = 1u << 6;
public static const uint FLOW_CONTINUOUS_REFLECTION = 1u << 7;
public static const uint FLOW_DELTA_TRANSMISSION = 1u << 8;
public static const uint FLOW_CONTINUOUS_TRANSMISSION = 1u << 9;
public static const uint FLOW_EMISSION = 1u << 10;
/// 金属选择权重可小于1；即使非金属端被证明为吸收，也必须保留其 null proposal。
public static const uint FLOW_METAL_MIX = 1u << 11;

public bool flow_has<let Flow : uint, let Bits : uint>()
{
    return (Flow & Bits) != 0u;
}

/// 整个闭包包含连续测度；光滑塑料的 diffuse 与 delta specular 可以同时存在。
public bool flow_continuous<let Flow : uint>()
{
    return flow_has<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                              FLOW_CONTINUOUS_TRANSMISSION>();
}

/// 与is_discrete_only相同的全闭包分类；Flow由资源世代证明，纯测度域不保留数值分类分支。
/// 混合金属的非金属null proposal仍保留原metalness判定，不能把它当成纯金属。
public bool is_discrete_only_flow<let Flow : uint>(Material material, float4 incident,
                                                   float4 transmitted)
{
    static const bool Delta = (Flow & (FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)) != 0;
    static const bool Transmission =
        (Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) != 0;
    static const bool Conductor = (Flow & FLOW_CONDUCTOR_REFLECTION) != 0;
    if (!Delta || !(Transmission || Conductor))
        return false;
    if (!flow_continuous<Flow>() && (Transmission || (Flow & FLOW_METAL_MIX) == 0))
        return true;
    return is_discrete_only(material, incident, transmitted);
}

internal bool flow_transmission<let Flow : uint>()
{
    return flow_has<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>();
}

#define FLOW_HAS(Key, Bits) (((Key) & (Bits)) != 0u)
#define FLOW_BASE(Key) FLOW_HAS(Key, FLOW_DIFFUSE | FLOW_SUBSURFACE)
#define FLOW_LAYER(Key) FLOW_HAS(Key, FLOW_DIELECTRIC_REFLECTION | FLOW_DIFFUSE | FLOW_SUBSURFACE)
#define FLOW_CONTINUOUS(Key)                                                                       \
    FLOW_HAS(Key, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION)
#define FLOW_LEGACY(Key)                                                                           \
    (FLOW_HAS(Key, FLOW_SOLID_TRANSMISSION) ? (FLOW_CONTINUOUS(Key) ? CAP_SOLID_DIELECTRIC : CAP_DELTA_SOLID_DIELECTRIC) : FLOW_HAS(Key, FLOW_THIN_TRANSMISSION) ? (FLOW_CONTINUOUS(Key) ? CAP_THIN_DIELECTRIC : CAP_DELTA_THIN_DIELECTRIC) : !FLOW_LAYER(Key) && !FLOW_HAS(Key, FLOW_METAL_MIX) ? (FLOW_CONTINUOUS(Key) ? CAP_CONDUCTOR : CAP_DELTA_CONDUCTOR) : FLOW_HAS(Key, FLOW_SUBSURFACE) ? CAP_OPAQUE_SUBSURFACE : CAP_OPAQUE_DIELECTRIC)

internal void require_flow<let Flow : uint>()
{
    static_assert((Flow & ~4095u) == 0u, "unknown OpenPBR flow bits");
    static_assert(FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION) ==
                      FLOW_HAS(Flow, FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION),
                  "reflection components require their measurement domains");
    static_assert(FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION) ==
                      FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION | FLOW_CONTINUOUS_TRANSMISSION),
                  "transmission topology requires its measurement domains");
    static_assert((Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) !=
                      (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION),
                  "solid and thin are distinct boundary contracts");
    static_assert(!FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION) ||
                      !FLOW_HAS(Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONDUCTOR_REFLECTION),
                  "opaque and transmission are disjoint in the supported subset");
    static_assert(!FLOW_HAS(Flow, FLOW_METAL_MIX) ||
                      (FLOW_HAS(Flow, FLOW_CONDUCTOR_REFLECTION) &&
                       !FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)),
                  "metal mixture requires an opaque conductor");
    static_assert(!FLOW_HAS(Flow, FLOW_CONDUCTOR_REFLECTION) || !FLOW_LAYER(Flow) ||
                      FLOW_HAS(Flow, FLOW_METAL_MIX),
                  "mixed metalness must retain its proposal control");
    static_assert(!FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION) ||
                      FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) ||
                      (FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION) &&
                       !FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION |
                                           FLOW_DELTA_REFLECTION)),
                  "matched transmission is delta only");
    static_assert(
        !FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) ||
            ((!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) || FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION)) &&
             (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) ==
              FLOW_HAS(Flow, FLOW_CONTINUOUS_TRANSMISSION))),
        "solid reflection/transmission domains disagree");
    static_assert(
        !FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION) || !FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) ||
            ((FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) == FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION)) &&
             (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) ==
              FLOW_HAS(Flow, FLOW_CONTINUOUS_TRANSMISSION))),
        "thin reflection/transmission domains disagree");
}

internal Material flow_material<let Flow : uint>(Material material)
{
    if (!flow_has<Flow, FLOW_SUBSURFACE>())
        material.surface.z = 0.0;
    else if (!flow_has<Flow, FLOW_DIFFUSE>())
        material.surface.z = 1.0;
    return material;
}

internal PrimeFullCompactOpaqueState flow_opaque<let Flow : uint, Provider>(Provider energy,
                                                                            Material material,
                                                                            float3 wi,
                                                                            float4 incident)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    material = flow_material<Flow>(material);
    float3 color = material.base_roughness.xyz;
    PrimeFullCompactOpaqueState state = {};
    if (FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) || !FLOW_LAYER(Flow))
        state = primeFullPtOpaqueStateImpl<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION),
                                           !FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION)>(
            energy, FLOW_BASE(Flow) || !FLOW_LAYER(Flow) ? color : float3(0.0),
            material.base_roughness.w, material.surface.x / incident.x, material.surface.z,
            !FLOW_LAYER(Flow), saturate(color), float3(1.0), wi);
    else
    {
        state.microfacet.alpha = FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION)
                                     ? float2(material.base_roughness.w * material.base_roughness.w)
                                     : float2(0.0);
        state.baseColor = color;
        state.subsurfaceWeight = material.surface.z;
        state.subsurfaceColor =
            FLOW_HAS(Flow, FLOW_SUBSURFACE) ? primeFullPtSubsurfaceColor(color) : color;
        state.subsurfaceAnisotropy = PRIME_FULL_PT_SUBSURFACE_ANISOTROPY;
        state.diffuseMix =
            primeFullCompactMakeMixState(material.surface.z, color, state.subsurfaceColor);
        state.dielectricFresnel.ior = 1.0;
        state.dielectricFresnel.energyIor = 1.0;
        state.dielectricFresnel.color = float3(1.0);
        state.dielectricMultipleScattering = float3(1.0, 1.0, 0.0);
        state.layer.transmittanceIn = float3(1.0);
        state.layer.specularSampleWeight = 0.0;
        state.layer.baseSampleWeight = any(color > 0.0) ? 1.0 : 0.0;
    }
    if (!FLOW_BASE(Flow) && FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION))
    {
        state.layer.baseSampleWeight = 0.0;
        state.layer.specularSampleWeight = state.dielectricMultipleScattering.z > 0.0 ? 1.0 : 0.0;
    }
    if (FLOW_LAYER(Flow) && flow_has<Flow, FLOW_CONDUCTOR_REFLECTION>())
    {
        state.metalness = material.surface.y;
        state.conductorFresnel.f0 = saturate(material.base_roughness.xyz);
        state.conductorFresnel.f82 = float3(1.0);
        state.conductorFresnel.f90 = float3(1.0);
        state.conductorFresnel.weight = 1.0;
        state.conductorFresnel.energyF0 = saturate(material.base_roughness.xyz);
        if (!flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>())
            state.conductorEss = 1.0;
        else
            state.conductorEss =
                flow_has<Flow, FLOW_DELTA_REFLECTION>() &&
                        primeFullOpenPbrMicrofacetEffectivelySmooth(state.microfacet)
                    ? 1.0
                    : primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
                          state.microfacet, wi.z, float3(1.0), float3(1.0))
                          .x;
    }
    if (FLOW_HAS(Flow, FLOW_METAL_MIX))
        state.metalness = material.surface.y;
    return state;
}

internal PrimeFullCompactOpaqueEvaluation
flow_layer_evaluate<let Flow : uint>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state,
                                     PrimeFullOpenPbrReflectiveEvalBase reflection)
{
    if (!FLOW_BASE(Flow))
    {
        PrimeFullCompactOpaqueEvaluation result = {};
        if (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION))
        {
            result.throughput =
                primeFullCompactDielectricEvalFromBase<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
                    reflection, state);
            if (state.layer.specularSampleWeight > 0.0)
                result.pdf = reflection.pdf;
        }
        return result;
    }
    if (FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) &&
        flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>() && !flow_has<Flow, FLOW_SUBSURFACE>())
        return primeFullCompactOpaqueLayerEvaluateFromBase<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
            wi, wo, state, reflection);
    PrimeFullOpenPbrThroughput base;
    float base_pdf;
    if (flow_has<Flow, FLOW_SUBSURFACE>() && flow_has<Flow, FLOW_DIFFUSE>())
    {
        base = primeFullCompactMixedDiffuseEval<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        base_pdf =
            primeFullCompactMixedDiffusePdf<true>(wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
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
    PrimeFullCompactOpaqueEvaluation result;
    if (!FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION))
    {
        result.throughput = base;
        result.pdf = state.layer.baseSampleWeight > 0.0 ? base_pdf : 0.0;
        return result;
    }
    PrimeFullOpenPbrThroughput specular = primeFullOpenPbrZeroThroughput();
    float specular_pdf = 0.0;
    if (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) && FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION))
    {
        specular = primeFullCompactDielectricEvalFromBase<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
            reflection, state);
        specular_pdf = reflection.pdf;
    }
    result.throughput = primeFullCompactLayerEval(base, specular, state.layer);
    result.pdf = state.layer.baseSampleWeight + state.layer.specularSampleWeight > 0.0
                     ? lerp(base_pdf, specular_pdf, state.layer.specularSampleWeight)
                     : 0.0;
    return result;
}

internal PrimeFullCompactOpaqueEvaluation
flow_opaque_evaluate<let Flow : uint>(float3 wi, float3 wo, PrimeFullCompactOpaqueState state)
{
    if (!FLOW_LAYER(Flow))
    {
        PrimeFullCompactOpaqueEvaluation result =
            specialized_opaque_evaluate<CAP_CONDUCTOR, !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
                wi, wo, state);
        if (FLOW_HAS(Flow, FLOW_METAL_MIX))
        {
            result.throughput.value *= state.metalness;
            result.pdf *= state.metalness;
        }
        return result;
    }
    PrimeFullOpenPbrReflectiveEvalBase reflection = {};
    if (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION))
        reflection = primeFullOpenPbrReflectiveEvalBaseImpl<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
            wi, wo, state.microfacet, PRIME_FULL_OPENPBR_FLAG_ALL,
            PRIME_FULL_OPENPBR_FLAG_REFLECTION);
    PrimeFullCompactOpaqueEvaluation first = flow_layer_evaluate<Flow>(wi, wo, state, reflection);
    if (!flow_has<Flow, FLOW_CONDUCTOR_REFLECTION>())
        return first;
    PrimeFullCompactOpaqueEvaluation second = {};
    if (flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>())
    {
        second.throughput =
            primeFullCompactConductorEvalFromBase<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
                reflection, state);
        second.pdf = reflection.pdf;
    }
    first.throughput =
        primeFullCompactMixEval(first.throughput, second.throughput, state.metalness);
    first.pdf = lerp(first.pdf, second.pdf, state.metalness);
    return first;
}

internal PrimeFullOpenPbrSample
flow_layer_sample<let Flow : uint>(float3 wi, float3 u, PrimeFullCompactOpaqueState state)
{
    if (!FLOW_BASE(Flow))
    {
        if (!(state.layer.specularSampleWeight > 0.0))
            return primeFullOpenPbrZeroSample();
        if (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION))
            return primeFullCompactDielectricSampleImpl<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
                wi, u, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        PrimeFullOpenPbrMicrofacet smooth = {};
        PrimeFullOpenPbrReflectiveSampleBase base = primeFullOpenPbrReflectiveSampleBase(
            wi, u, smooth, PRIME_FULL_OPENPBR_FLAG_ALL, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
        PrimeFullOpenPbrSample result = base.bsdfSample;
        result.throughput.value *=
            primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine);
        return result;
    }
    if (FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) &&
        flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>() && !flow_has<Flow, FLOW_SUBSURFACE>())
        return primeFullCompactOpaqueLayerSampleImpl<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
            wi, u, state, false);
    if (state.layer.baseSampleWeight + state.layer.specularSampleWeight <= 0.0)
        return primeFullOpenPbrZeroSample();
    if (FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) && u.z <= state.layer.specularSampleWeight &&
        state.layer.specularSampleWeight > 0.0)
    {
        if (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION))
        {
            PrimeFullOpenPbrSample result =
                primeFullCompactDielectricSampleImpl<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
                    wi, float3(u.xy, u.z / state.layer.specularSampleWeight), state,
                    PRIME_FULL_OPENPBR_FLAG_ALL);
            result.pdf *= state.layer.specularSampleWeight;
            return result;
        }
        PrimeFullOpenPbrMicrofacet smooth = {};
        PrimeFullOpenPbrReflectiveSampleBase base = primeFullOpenPbrReflectiveSampleBase(
            wi, u, smooth, PRIME_FULL_OPENPBR_FLAG_ALL, PRIME_FULL_OPENPBR_FLAG_REFLECTION);
        PrimeFullOpenPbrSample result = base.bsdfSample;
        result.throughput.value *=
            primeFullOpenPbrSpecularFresnelUnpolarized(state.dielectricFresnel, base.microCosine);
        result.pdf *= state.layer.specularSampleWeight;
        return result;
    }
    float3 random =
        FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION)
            ? float3(u.xy, (u.z - state.layer.specularSampleWeight) / state.layer.baseSampleWeight)
            : u;
    PrimeFullOpenPbrSample result;
    if (flow_has<Flow, FLOW_SUBSURFACE>() && flow_has<Flow, FLOW_DIFFUSE>())
    {
        if (state.subsurfaceWeight == 0.0)
            result = primeFullCompactLambertSample(wi, random, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        else if (state.subsurfaceWeight == 1.0)
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
    if (FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION))
    {
        result.throughput =
            primeFullOpenPbrThroughputScale(result.throughput, state.layer.transmittanceIn);
        result.pdf *= state.layer.baseSampleWeight;
    }
    return result;
}

internal PrimeFullOpenPbrSample
flow_opaque_sample<let Flow : uint>(float3 wi, float3 u, PrimeFullCompactOpaqueState state)
{
    if (!FLOW_LAYER(Flow))
    {
        if (FLOW_HAS(Flow, FLOW_METAL_MIX) && u.z >= state.metalness)
            return primeFullOpenPbrZeroSample();
        PrimeFullOpenPbrSample result;
        if (!FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION))
            result = primeFullCompactConductorDiscreteSample(
                wi, primeFullCompactOpaqueDiscreteStateFromFull(state),
                PRIME_FULL_OPENPBR_FLAG_ALL);
        else
            result =
                specialized_opaque_sample<CAP_CONDUCTOR, !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
                    wi, FLOW_HAS(Flow, FLOW_METAL_MIX) ? float3(u.xy, u.z / state.metalness) : u,
                    state);
        if (FLOW_HAS(Flow, FLOW_METAL_MIX))
        {
            if (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) && state.metalness < 1.0 &&
                primeFullOpenPbrHasSample(result) &&
                (!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) ||
                 primeFullOpenPbrIsNonDelta(result.throughput.flags)))
            {
                PrimeFullCompactOpaqueEvaluation evaluated = specialized_opaque_evaluate<
                    CAP_CONDUCTOR, !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(wi, result.wo, state);
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
    PrimeFullOpenPbrSample result;
    if (flow_has<Flow, FLOW_CONDUCTOR_REFLECTION>())
    {
        float m = state.metalness;
        if (u.z < m)
        {
            if (!flow_has<Flow, FLOW_CONTINUOUS_REFLECTION>())
                result = primeFullCompactConductorDiscreteSample(
                    wi, primeFullCompactOpaqueDiscreteStateFromFull(state),
                    PRIME_FULL_OPENPBR_FLAG_ALL);
            else
                result =
                    primeFullCompactConductorSampleImpl<!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
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
    PrimeFullCompactOpaqueEvaluation evaluated = flow_opaque_evaluate<Flow>(wi, result.wo, state);
    if (flow_has<Flow, FLOW_CONDUCTOR_REFLECTION>() && state.metalness > 0.0 &&
        state.metalness < 1.0)
        result.throughput = evaluated.throughput;
    else
        result.throughput.value = evaluated.throughput.value;
    result.pdf = evaluated.pdf;
    return result;
}

/// 编译期能力的连续支持域；不读取纹理。动态 TIR、有效方向等仍由实际闭包处理。
public bool may_evaluate_flow<let Flow : uint>(Material material, float3 normal, float3 view,
                                               float3 direction, float4 incident,
                                               float4 transmitted)
{
    require_flow<Flow>();
    if (!flow_continuous<Flow>())
        return false;
    if (flow_transmission<Flow>())
        return true;
    if (FLOW_HAS(Flow, FLOW_SUBSURFACE))
    {
        bool dielectric_base = !FLOW_HAS(Flow, FLOW_METAL_MIX) || material.surface.y < 1.0;
        bool subsurface = !FLOW_HAS(Flow, FLOW_DIFFUSE) || material.surface.z > 0.0;
        if (dielectric_base && subsurface)
            return true;
    }
    if (!FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) && FLOW_HAS(Flow, FLOW_METAL_MIX) &&
        material.surface.y == 1.0)
        return false;
    return may_evaluate_specialized<CAP_OPAQUE_DIELECTRIC>(material, normal, view, direction,
                                                           incident, transmitted);
}

/// 与未裁剪 evaluate 相同的完整连续响应与 marginal PDF；Flow 必须覆盖全部可达分量。
public Evaluation evaluate_flow<let Flow : uint, Provider>(Provider energy, Material material,
                                                           float3 normal, float3 view,
                                                           float3 direction, float4 incident,
                                                           float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_flow<Flow>();
    if (!flow_continuous<Flow>())
        return { float3(0.0), 0.0 };
    if (flow_transmission<Flow>())
    {
        if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) &&
            FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION))
        {
            float ior = incident.x > transmitted.x ? incident.x / transmitted.x
                                                   : transmitted.x / incident.x;
            if (ior == 1.0)
                return { float3(0.0), 0.0 };
            return evaluate_solid_continuous(energy, material, normal, view, direction, incident,
                                             transmitted);
        }
        if (FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION) &&
            !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION))
        {
            PrimeFullOpenPbrOnb onb =
                specialized_frame<CAP_THIN_DIELECTRIC>(normal, view, incident, transmitted);
            float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
            float3 wo = primeFullOpenPbrOnbToLocal(onb, direction);
            PrimeFullCompactTransmissionState state = primeFullPtTransmissionState(
                energy, material.base_roughness.w, transmitted.x / incident.x, true,
                transmitted.yzw * material.surface.w, wi);
            PrimeFullCompactTransmissionEvaluation evaluated =
                primeFullCompactTransmissionEvaluateRaw<true>(wi, wo, state,
                                                              PRIME_FULL_OPENPBR_FLAG_ALL);
            return specialized_evaluation(evaluated.throughput, evaluated.pdf);
        }
        if (flow_has<Flow, FLOW_SOLID_TRANSMISSION>() &&
            !flow_has<Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION>())
            return evaluate_solid_continuous(energy, material, normal, view, direction, incident,
                                             transmitted);
        return evaluate_specialized<FLOW_LEGACY(Flow)>(energy, material, normal, view, direction,
                                                       incident, transmitted);
    }
    PrimeFullOpenPbrOnb onb =
        specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    float3 wo = primeFullOpenPbrOnbToLocal(onb, direction);
    if (!flow_has<Flow, FLOW_SUBSURFACE>() && wo.z < 0.0)
        return { float3(0.0), 0.0 };
    PrimeFullCompactOpaqueEvaluation evaluated =
        flow_opaque_evaluate<Flow>(wi, wo, flow_opaque<Flow>(energy, material, wi, incident));
    return specialized_evaluation < FLOW_HAS(Flow, FLOW_SUBSURFACE) &&
           !FLOW_HAS(Flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION) >
               (evaluated.throughput, evaluated.pdf);
}

internal Sample flow_matched_solid(float3 normal, float3 view, float4 incident, float4 transmitted)
{
    PrimeFullOpenPbrOnb onb =
        specialized_frame<CAP_DELTA_SOLID_DIELECTRIC>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    PrimeFullOpenPbrRefractResult refracted =
        primeFullOpenPbrDielectricRefract(1.0, -wi, float3(0.0, 0.0, 1.0));
    Sample result = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (refracted.valid != 0u && wi.z * refracted.wo.z < 0.0)
    {
        float3 direction = primeFullOpenPbrOnbToWorld(onb, refracted.wo);
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
public Sample sample_flow<let Flow : uint, Provider>(Provider energy, Material material,
                                                     float3 normal, float3 view, float3 u,
                                                     float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    require_flow<Flow>();
    if (!FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION | FLOW_DIFFUSE |
                            FLOW_SUBSURFACE | FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION))
    {
        Sample result = {};
        result.relative_eta = 1.0;
        result.medium = incident;
        return result;
    }
    if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION))
        return flow_matched_solid(normal, view, incident, transmitted);
    if (FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION))
    {
        Sample result = {};
        result.relative_eta = 1.0;
        result.medium = incident;
        float cosine = abs(dot(normal, view));
        float3 response =
            cosine > 0.0 ? exp(-transmitted.yzw * material.surface.w / cosine) : float3(0.0);
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
        ((!FLOW_LAYER(Flow) && !FLOW_HAS(Flow, FLOW_METAL_MIX)) || flow_transmission<Flow>()))
        return sample_specialized<FLOW_LEGACY(Flow)>(energy, material, normal, view, u, incident,
                                                     transmitted);
    if (!FLOW_BASE(Flow) &&
        !FLOW_HAS(Flow, FLOW_CONDUCTOR_REFLECTION | FLOW_SOLID_TRANSMISSION |
                            FLOW_THIN_TRANSMISSION | FLOW_CONTINUOUS_REFLECTION))
    {
        PrimeFullOpenPbrOnb onb =
            specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
        float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
        PrimeFullOpenPbrSample result = primeFullOpenPbrZeroSample();
        PrimeFullOpenPbrSpecularFresnel fresnel;
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
        if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) &&
            FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION))
        {
            float ior = incident.x > transmitted.x ? incident.x / transmitted.x
                                                   : transmitted.x / incident.x;
            if (ior != 1.0)
                return sample_solid_continuous(energy, material, normal, view, u, incident,
                                               transmitted);
            return flow_matched_solid(normal, view, incident, transmitted);
        }
        if (FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION) &&
            !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION))
        {
            PrimeFullOpenPbrOnb onb =
                specialized_frame<CAP_THIN_DIELECTRIC>(normal, view, incident, transmitted);
            float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
            PrimeFullCompactTransmissionState state = primeFullPtTransmissionState(
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
        return sample_specialized<FLOW_LEGACY(Flow)>(energy, material, normal, view, u, incident,
                                                     transmitted);
    }
    PrimeFullOpenPbrOnb onb =
        specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    PrimeFullOpenPbrSample sampled =
        flow_opaque_sample<Flow>(wi, u, flow_opaque<Flow>(energy, material, wi, incident));
    return specialized_result<FLOW_LEGACY(Flow), !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
        sampled, onb, view, incident, transmitted);
}

internal Evaluation flow_transmission_evaluate_prepared<let Flow : uint>(
    float3 wi, float3 wo, PrimeFullCompactTransmissionState state)
{
    PrimeFullCompactTransmissionEvaluation result;
    if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION))
        result = primeFullCompactTransmissionEvaluateRawSolidContinuous(
            wi, wo, state, PRIME_FULL_OPENPBR_FLAG_ALL);
    else
        result = primeFullCompactTransmissionEvaluateRaw<!FLOW_HAS(
            Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)>(wi, wo, state,
                                                                    PRIME_FULL_OPENPBR_FLAG_ALL);
    return specialized_evaluation(result.throughput, result.pdf);
}

internal PrimeFullOpenPbrSample flow_transmission_sample_prepared<let Flow : uint>(
    float3 wi, float3 u, PrimeFullCompactTransmissionState state)
{
    if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION))
        return primeFullCompactTransmissionSampleRawSolidContinuous(wi, u, state,
                                                                    PRIME_FULL_OPENPBR_FLAG_ALL);
    return primeFullCompactTransmissionSampleRaw<!FLOW_HAS(
        Flow, FLOW_DELTA_REFLECTION | FLOW_DELTA_TRANSMISSION)>(wi, u, state,
                                                                PRIME_FULL_OPENPBR_FLAG_ALL);
}

/// 按方向即时消费连续评估；direction=0/1 对应 first/second，只回调掩码请求的方向。
/// 回调在共享闭包状态有效期内执行，不得发起 callable 或主查询。
public interface IFlowEvaluationConsumer
{
    [mutating]
    void consume(uint direction, Evaluation evaluated);
};

/// 单一顶点共享 ONB 和状态；先依次消费评估，最后生成散射。
/// 所有可见性查询须由调用方先完成；昂贵外部光照求值应留在本函数返回之后。
public Sample shade_flow_consume<let Flow : uint, Provider, Consumer, let Directions : uint = 3>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted,
    inout Consumer consumer)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
    where Consumer : IFlowEvaluationConsumer
{
    require_flow<Flow>();
    static_assert((Directions & ~3u) == 0u, "unknown evaluation direction mask");
    Sample result = {};
    result.relative_eta = 1.0;
    result.medium = incident;
    if (!flow_continuous<Flow>())
    {
        if ((Directions & 1u) != 0u && (evaluation_mask & 1u) != 0u)
            consumer.consume(0u, evaluate_flow<Flow>(energy, material, normal, view, first,
                                                     incident, transmitted));
        if ((Directions & 2u) != 0u && (evaluation_mask & 2u) != 0u)
            consumer.consume(1u, evaluate_flow<Flow>(energy, material, normal, view, second,
                                                     incident, transmitted));
        if (continuation)
            result = sample_flow<Flow>(energy, material, normal, view, u, incident, transmitted);
        return result;
    }
    if ((evaluation_mask & Directions) == 0u && !continuation)
        return result;
    if (flow_transmission<Flow>())
    {
        float ior = FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION) ? transmitted.x / incident.x
                    : incident.x > transmitted.x           ? incident.x / transmitted.x
                                                           : transmitted.x / incident.x;
        if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) && !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) &&
            FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION) && ior == 1.0)
        {
            if (continuation)
                result = flow_matched_solid(normal, view, incident, transmitted);
            return result;
        }
        PrimeFullOpenPbrOnb onb =
            specialized_frame<FLOW_LEGACY(Flow)>(normal, view, incident, transmitted);
        float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
        PrimeFullCompactTransmissionState state = primeFullPtTransmissionState(
            energy, material.base_roughness.w, ior, FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION),
            transmitted.yzw * material.surface.w, wi);
        if (FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION))
            state.entering = 1u;
        if ((Directions & 1u) != 0u && (evaluation_mask & 1u) != 0u)
            consumer.consume(0u, flow_transmission_evaluate_prepared<Flow>(
                                     wi, primeFullOpenPbrOnbToLocal(onb, first), state));
        if ((Directions & 2u) != 0u && (evaluation_mask & 2u) != 0u)
            consumer.consume(1u, flow_transmission_evaluate_prepared<Flow>(
                                     wi, primeFullOpenPbrOnbToLocal(onb, second), state));
        if (continuation)
            result =
                specialized_result<FLOW_LEGACY(Flow), !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION |
                                                                          FLOW_DELTA_TRANSMISSION)>(
                    flow_transmission_sample_prepared<Flow>(wi, u, state), onb, view, incident,
                    transmitted);
        return result;
    }
    PrimeFullOpenPbrOnb onb =
        specialized_frame<CAP_OPAQUE_DIELECTRIC>(normal, view, incident, transmitted);
    float3 wi = primeFullOpenPbrOnbToLocal(onb, view);
    PrimeFullCompactOpaqueState state = flow_opaque<Flow>(energy, material, wi, incident);
    if ((Directions & 1u) != 0u && (evaluation_mask & 1u) != 0u)
    {
        PrimeFullCompactOpaqueEvaluation evaluated =
            flow_opaque_evaluate<Flow>(wi, primeFullOpenPbrOnbToLocal(onb, first), state);
        consumer.consume(0u, specialized_evaluation < FLOW_HAS(Flow, FLOW_SUBSURFACE) &&
                                 !FLOW_HAS(Flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION |
                                                     FLOW_CONDUCTOR_REFLECTION) >
                                     (evaluated.throughput, evaluated.pdf));
    }
    if ((Directions & 2u) != 0u && (evaluation_mask & 2u) != 0u)
    {
        PrimeFullCompactOpaqueEvaluation evaluated =
            flow_opaque_evaluate<Flow>(wi, primeFullOpenPbrOnbToLocal(onb, second), state);
        consumer.consume(1u, specialized_evaluation < FLOW_HAS(Flow, FLOW_SUBSURFACE) &&
                                 !FLOW_HAS(Flow, FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION |
                                                     FLOW_CONDUCTOR_REFLECTION) >
                                     (evaluated.throughput, evaluated.pdf));
    }
    if (continuation)
        result = specialized_result<FLOW_LEGACY(Flow), !FLOW_HAS(Flow, FLOW_DELTA_REFLECTION)>(
            flow_opaque_sample<Flow>(wi, u, state), onb, view, incident, transmitted);
    return result;
}

internal struct FlowBatchCollector : IFlowEvaluationConsumer
{
    Batch value;

    [mutating]
    void consume(uint direction, Evaluation evaluated)
    {
        if (direction == 0u)
            value.first = evaluated;
        else
            value.second = evaluated;
    }
};

/// 收集两方向评估与最后的散射；即时消费方使用 shade_flow_consume，避免保留完整 Batch。
public Batch shade_flow<let Flow : uint, Provider, let Directions : uint = 3>(
    Provider energy, Material material, float3 normal, float3 view, float3 first, float3 second,
    uint evaluation_mask, float3 u, bool continuation, float4 incident, float4 transmitted)
    where Provider : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    FlowBatchCollector collector = {};
    Sample scattered = shade_flow_consume<Flow, Provider, FlowBatchCollector, Directions>(
        energy, material, normal, view, first, second, evaluation_mask, u, continuation, incident,
        transmitted, collector);
    collector.value.scattered = scattered;
    return collector.value;
}
}
