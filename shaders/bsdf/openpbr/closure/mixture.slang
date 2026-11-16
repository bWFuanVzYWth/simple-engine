// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 底层与金属混合；黑色非金属端保留null提案，未使用的共享反射约束延迟实例化。
module mixture;
import bsdf.openpbr.closure.contracts;
import bsdf.openpbr.closure.reflection;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.math.spectrum;
import bsdf.openpbr.opaque.lobes;
import bsdf.openpbr.service.support.conversion;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.opaque.state;
import bsdf.openpbr.math.microfacet_energy;

namespace openpbr
{
namespace closure
{
public struct BaseClosure<Base : IBase> : IClosure
{
    public static func has_delta()->bool
    {
        return false;
    }
    public typedef Base.State State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State
    {
        return Base.prepare(material.base_roughness.xyz, material.surface.z);
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation
    {
        var evaluated : LocalEvaluation = Base.evaluate(wi, wo, state);
        if (!any(Base.energy(wi, state) > 0))
            evaluated.pdf = 0;
        return evaluated;
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal
    {
        return any(Base.energy(wi, state) > 0) ? Base.sample(wi, random, state)
                                               : kernel::empty_proposal();
    }
    public static func delta_response(wi: float3, state: State)->float3
    {
        return 0;
    }
    public static func delta_pdf(state: State)->float
    {
        return 0;
    }
    public static func marginal_flags(state: State)->bool
    {
        return false;
    }
    public static func marginal_required(state: State)->bool
    {
        return Base.marginal_required();
    }
};
public struct MetalBlendState<Dielectric : IClosure, Metal : IClosure>
{
    public var dielectric : Dielectric.State;
    public var conductor : Metal.State;
    public var metalness : float;
};
public struct MetalBlend<Dielectric : IClosure, Metal : IClosure> : IClosure
{
    public static func has_delta()->bool
    {
        return Dielectric.has_delta() || Metal.has_delta();
    }
    public typedef MetalBlendState<Dielectric, Metal> State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State
    {
        return { Dielectric.prepare(energy, material, wi, incident),
                 Metal.prepare(energy, material, wi, incident), material.surface.y };
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation
    {
        let a : LocalEvaluation = Dielectric.evaluate(wi, wo, state.dielectric);
        let b : LocalEvaluation = Metal.evaluate(wi, wo, state.conductor);
        return { kernel::mix_eval(a.response, b.response, state.metalness),
                 lerp(a.pdf, b.pdf, state.metalness) };
    }
    public static func delta_response(wi: float3, state: State)->float3
    {
        return lerp(Dielectric.delta_response(wi, state.dielectric),
                    Metal.delta_response(wi, state.conductor), state.metalness);
    }
    public static func delta_pdf(state: State)->float
    {
        return state.metalness + (1 - state.metalness) * Dielectric.delta_pdf(state.dielectric);
    }
    public static func marginal_flags(state: State)->bool
    {
        return state.metalness > 0 && state.metalness < 1;
    }
    public static func marginal_required(state: State)->bool
    {
        return state.metalness < 1;
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal
    {
        var proposal : Proposal;
        if (random.z < state.metalness)
            proposal =
                Metal.sample(wi, float3(random.xy, random.z / state.metalness), state.conductor);
        else
            proposal = Dielectric.sample(
                wi, float3(random.xy, (random.z - state.metalness) / (1 - state.metalness)),
                state.dielectric);
        if (kernel::has_sample(proposal) && marginal_flags(state) && Metal.has_delta() &&
            kernel::is_delta(proposal.response.flags))
        {
            proposal.response = { delta_response(wi, state), kernel::LOBE_DELTA_REFLECTION };
            proposal.pdf = delta_pdf(state);
        }
        return proposal;
    }
};
/// 分数金属与介电反射共用GGX；保留一般混合的采样与delta规则。
public struct SharedMetalBlend<Measure : IMeasure, Dielectric : IReflectionClosure> : IClosure
{
    public typedef MetalBlend<Dielectric, Conductor<Measure>> Mixture;
    public typedef MetalBlendState<Dielectric, Conductor<Measure>> State;
    public static func has_delta()->bool
    {
        return Measure.has_delta();
    }
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State
    {
        return Mixture.prepare(energy, material, wi, incident);
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation
    {
        let reflection : kernel::ReflectionEvaluation =
                             Measure.evaluate(wi, wo, state.conductor.geometry);
        let dielectric : LocalEvaluation =
                             Dielectric.from_reflection(wi, wo, state.dielectric, reflection);
        let metal : LocalEvaluation =
                        Conductor<Measure>.from_reflection(wi, wo, state.conductor, reflection);
        return { kernel::mix_eval(dielectric.response, metal.response, state.metalness),
                 lerp(dielectric.pdf, metal.pdf, state.metalness) };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal
    {
        return Mixture.sample(wi, random, state);
    }
    public static func delta_response(wi: float3, state: State)->float3
    {
        return Mixture.delta_response(wi, state);
    }
    public static func delta_pdf(state: State)->float
    {
        return Mixture.delta_pdf(state);
    }
    public static func marginal_flags(state: State)->bool
    {
        return Mixture.marginal_flags(state);
    }
    public static func marginal_required(state: State)->bool
    {
        return Mixture.marginal_required(state);
    }
};
public struct NullMetalState<Metal : IClosure>
{
    public var metal : Metal.State;
    public var weight : float;
};
/// 黑色非金属端仍保留null proposal，不重新归一化金属提案。
public struct NullMetal<Metal : IClosure> : IClosure
{
    public static func has_delta()->bool
    {
        return Metal.has_delta();
    }
    public typedef NullMetalState<Metal> State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State
    {
        return { Metal.prepare(energy, material, wi, incident), material.surface.y };
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation
    {
        var evaluated : LocalEvaluation = Metal.evaluate(wi, wo, state.metal);
        evaluated.response.value *= state.weight;
        evaluated.pdf *= state.weight;
        return evaluated;
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal
    {
        if (random.z >= state.weight)
            return kernel::empty_proposal();
        var proposal
            : Proposal = Metal.sample(wi, float3(random.xy, random.z / state.weight), state.metal);
        proposal.response.value *= state.weight;
        proposal.pdf *= state.weight;
        if (state.weight < 1 &&
            (!Metal.has_delta() || kernel::is_non_delta(proposal.response.flags)))
            proposal.response.flags |= kernel::LOBE_DIFFUSE_REFLECTION;
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3
    {
        return Metal.delta_response(wi, state.metal) * state.weight;
    }
    public static func delta_pdf(state: State)->float
    {
        return state.weight;
    }
    public static func marginal_flags(state: State)->bool
    {
        return false;
    }
    public static func marginal_required(state: State)->bool
    {
        return Metal.marginal_required(state.metal);
    }
};
/// 先选择组合，再实例化其闭包约束；未使用的共享反射组合无需满足额外约束。
public struct MetalMixture<let shared : bool, Measure : IMeasure, Base : IClosure>
{
};
public extension<Measure : IMeasure, Base : IReflectionClosure> MetalMixture<true, Measure, Base>
{
    public typedef SharedMetalBlend<Measure, Base> Type;
};
public extension<Measure : IMeasure, Base : IClosure> MetalMixture<false, Measure, Base>
{
    public typedef MetalBlend<Base, Conductor<Measure>> Type;
};
}
}
