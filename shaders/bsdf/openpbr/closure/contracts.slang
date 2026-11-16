// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 局部坐标闭包的状态、响应、提案与测度契约；采样完成时恢复需要的完整边缘密度。
module contracts;
__exported import bsdf.openpbr.contract.surface;
__exported import bsdf.openpbr.contract.sample;
__exported import bsdf.openpbr.contract.flags;
__exported import bsdf.openpbr.opaque.state;
__exported import bsdf.openpbr.math.microfacet_distribution;
__exported import bsdf.openpbr.math.microfacet_energy;
__exported import bsdf.openpbr.service.support.conversion;

namespace openpbr
{
namespace closure
{
public typedef kernel::LocalEvaluation LocalEvaluation;
public typedef kernel::Proposal Proposal;
public struct Empty
{
};

public interface IBase
{
    associatedtype State;
    static func prepare(color: float3, weight: float)->State;
    static func marginal_required()->bool;
    static func energy(wi: float3, state: State)->float3;
    static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation;
    static func sample(wi: float3, random: float3, state: State)->Proposal;
};
public interface IMeasure
{
    associatedtype State;
    static func has_delta()->bool;
    static func prepare(roughness: float)->State;
    static func prepare_alpha(alpha: float)->State;
    static func distribution(state: State)->kernel::Microfacet;
    static func delta(state: State)->bool;
    static func evaluate(wi: float3, wo: float3, state: State)->kernel::ReflectionEvaluation;
    static func sample(wi: float3, random: float3, state: State)->kernel::ReflectionProposal;
    static func conductor_energy(wi: float3, state: State)->float;
    static func dielectric_energy<EnergyLut : IEnergyLut>(energy: EnergyLut, wi: float3, ior: float,
                                                          state: State, roughness: float)
        ->float3;
};
public interface IClosure
{
    associatedtype State;
    static func has_delta()->bool;
    static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut, material: openpbr::Material,
                                                wi: float3, incident: float)
        ->State;
    static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation;
    /// 单个proposal；外层组合在连续样本上统一恢复marginal响应和PDF。
    static func sample(wi: float3, random: float3, state: State)->Proposal;
    static func delta_response(wi: float3, state: State)->float3;
    static func delta_pdf(state: State)->float;
    static func marginal_flags(state: State)->bool;
    static func marginal_required(state: State)->bool;
};
/// 共享同一微表面分布的闭包只组合响应，不重复计算GGX几何项。
public interface IReflectionClosure : IClosure
{
    static func from_reflection(wi: float3, wo: float3, state: State,
                                reflection: kernel::ReflectionEvaluation)
        ->LocalEvaluation;
};
public func sample_closure<Closure : IClosure>(wi: float3, random: float3, state: Closure.State)
    ->Proposal
{
    var proposal : Proposal = Closure.sample(wi, random, state);
    if (Closure.marginal_required(state) && kernel::has_sample(proposal) &&
        (!Closure.has_delta() || kernel::is_non_delta(proposal.response.flags)))
    {
        let evaluated : LocalEvaluation = Closure.evaluate(wi, proposal.wo, state);
        if (Closure.marginal_flags(state))
            proposal.response = evaluated.response;
        else
            proposal.response.value = evaluated.response.value;
        proposal.pdf = evaluated.pdf;
    }
    return proposal;
};
}
}
