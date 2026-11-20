// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 导体、介电反射与介电底层；复用已求出的反射几何，保留能量补偿与提案权重。
module reflection;
import openpbr.closure.contracts;
import openpbr.contract.flags;
import openpbr.math.spectrum;
import openpbr.math.fresnel;
import openpbr.math.microfacet_measure;
import openpbr.opaque.lobes;
import openpbr.contract.sample;
import openpbr.opaque.state;
import openpbr.math.microfacet_energy;

namespace openpbr {
namespace closure {
public struct ConductorState<Measure : IMeasure> {
    public var geometry : Measure.State;
    public var color : float3;
    public var multiple_scattering_ratio : float;
};
public func conductor_fresnel(color: float3)->kernel::ConductorFresnel {
    return { color, float3(1), float3(1), 1, color };
}
func conductor_response(color: float3, cosine: float)->float3 {
    return saturate(lerp(color, float3(1), kernel::pow5(1 - cosine)));
}
public struct Conductor<Measure : IMeasure> : IReflectionClosure {
    public static func has_delta()->bool {
        return Measure.has_delta();
    }
    public typedef ConductorState<Measure> State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        let geometry : Measure.State = Measure.prepare(material.base_roughness.w);
        let ess = Measure.conductor_energy(wi, geometry);
        return { geometry, saturate(material.base_roughness.xyz), (1 - ess) / ess };
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return from_reflection(wi, wo, state, Measure.evaluate(wi, wo, state.geometry));
    }
    public static func from_reflection(wi: float3, wo: float3, state: State,
                                       reflection: kernel::ReflectionEvaluation)
        ->LocalEvaluation {
        if (reflection.flags == 0)
            return {};
        var value
            : float3 = conductor_response(state.color, reflection.micro_cosine) * reflection.factor;
        value *= 1 + state.color * state.multiple_scattering_ratio;
        return { { value, reflection.flags }, reflection.pdf };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        let reflection : kernel::ReflectionProposal = Measure.sample(wi, random, state.geometry);
        var proposal : Proposal = reflection.proposal;
        proposal.response.value *= conductor_response(state.color, reflection.micro_cosine);
        if (!Measure.has_delta() || kernel::is_non_delta(proposal.response.flags))
            proposal.response.value *= 1 + state.color * state.multiple_scattering_ratio;
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return conductor_response(state.color, wi.z);
    }
    public static func delta_pdf(state: State)->float {
        return 1;
    }
    public static func marginal_flags(state: State)->bool {
        return false;
    }
    public static func marginal_required(state: State)->bool {
        return false;
    }
};
public struct LayerState<Measure : IMeasure, Base : IBase> {
    public var geometry : Measure.State;
    public var base : Base.State;
    public var energy_ior : float;
    /// x=反射补偿，y=透射补偿，z=方向反射能量；保持原float3状态。
    public var energy : float3;
    public var mixture : kernel::LayerState;
};
public func dielectric_fresnel(ior: float)->kernel::SpecularFresnel {
    return { ior, kernel::ior_adjustment(ior, 1), float3(1) };
}
func dielectric_reflectance(energy_ior: float, cosine: float)->float3 {
    return float3(kernel::dielectric_reflectance(energy_ior, cosine));
}

/// 不透明光滑界面在镜向上的离散响应。cosine 为视线与着色法线夹角的绝对余弦。
public func opaque_delta_reflectance(material: openpbr::Material, cosine: float,
                                     incident_ior: float)
    ->float3 {
    let dielectric =
        kernel::specular_reflectance(dielectric_fresnel(material.surface.x / incident_ior), cosine);
    let metal = conductor_response(saturate(material.base_roughness.xyz), cosine);
    return lerp(dielectric, metal, material.surface.y);
}

public struct DeltaDielectric : IClosure {
    public static func has_delta()->bool {
        return true;
    }
    public typedef float State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        return material.surface.x / incident;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return {};
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        var proposal : Proposal = kernel::empty_proposal();
        if (wi.z > 0 && state != 1) {
            proposal.wo = reflect(-wi, float3(0, 0, 1));
            proposal.response = { delta_response(wi, state), kernel::LOBE_DELTA_REFLECTION };
            proposal.pdf = 1;
        }
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return kernel::specular_reflectance(dielectric_fresnel(state), wi.z);
    }
    public static func delta_pdf(state: State)->float {
        return state != 1 ? 1 : 0;
    }
    public static func marginal_flags(state: State)->bool {
        return false;
    }
    public static func marginal_required(state: State)->bool {
        return false;
    }
};
public struct SpecularState<Measure : IMeasure> {
    public var geometry : Measure.State;
    public var energy_ior : float;
    public var reflection_compensation : float;
    public var available : bool;
};
public struct Specular<Measure : IMeasure> : IReflectionClosure {
    public static func has_delta()->bool {
        return Measure.has_delta();
    }
    public typedef SpecularState<Measure> State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        let geometry : Measure.State = Measure.prepare(material.base_roughness.w);
        let ior : float = kernel::ior_adjustment(material.surface.x / incident, 1);
        let directional_energy : float3 = Measure.dielectric_energy(energy, wi, ior, geometry,
                                                                    material.base_roughness.w);
        return { geometry, ior, directional_energy.x, directional_energy.z > 0 };
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return from_reflection(wi, wo, state, Measure.evaluate(wi, wo, state.geometry));
    }
    public static func from_reflection(wi: float3, wo: float3, state: State,
                                       reflection: kernel::ReflectionEvaluation)
        ->LocalEvaluation {
        if (reflection.flags == 0)
            return {};
        return { { dielectric_reflectance(state.energy_ior, reflection.micro_cosine) *
                       reflection.factor * state.reflection_compensation,
                   reflection.flags },
                 state.available ? reflection.pdf : 0 };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        if (!state.available)
            return kernel::empty_proposal();
        let reflection : kernel::ReflectionProposal = Measure.sample(wi, random, state.geometry);
        var proposal : Proposal = reflection.proposal;
        proposal.response.value *=
            dielectric_reflectance(state.energy_ior, reflection.micro_cosine);
        if (!Measure.has_delta() || kernel::is_non_delta(proposal.response.flags))
            proposal.response.value *= state.reflection_compensation;
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return dielectric_reflectance(state.energy_ior, wi.z);
    }
    public static func delta_pdf(state: State)->float {
        return state.available ? 1 : 0;
    }
    public static func marginal_flags(state: State)->bool {
        return false;
    }
    public static func marginal_required(state: State)->bool {
        return false;
    }
};
public struct DielectricLayer<Measure : IMeasure, Base : IBase> : IReflectionClosure {
    public static func has_delta()->bool {
        return Measure.has_delta();
    }
    public typedef LayerState<Measure, Base> State;
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        var state : State;
        state.geometry = Measure.prepare(material.base_roughness.w);
        state.base = Base.prepare(material.base_roughness.xyz, material.surface.z);
        state.energy_ior = kernel::ior_adjustment(material.surface.x / incident, 1);
        state.energy = Measure.dielectric_energy(energy, wi, state.energy_ior, state.geometry,
                                                 material.base_roughness.w);
        state.mixture =
            kernel::make_layer_state(Base.energy(wi, state.base), float3(1 - state.energy.z),
                                     wi.z >= 0 ? float3(state.energy.z) : float3(0));
        return state;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return from_reflection(wi, wo, state, Measure.evaluate(wi, wo, state.geometry));
    }
    public static func from_reflection(wi: float3, wo: float3, state: State,
                                       reflected: kernel::ReflectionEvaluation)
        ->LocalEvaluation {
        let base : LocalEvaluation = Base.evaluate(wi, wo, state.base);
        return from_base(base, state, reflected);
    }
    public static func from_base(base: LocalEvaluation, state: State,
                                 reflected: kernel::ReflectionEvaluation)
        ->LocalEvaluation {
        var specular : kernel::Response = kernel::empty_response();
        if (reflected.flags != 0)
            specular = { dielectric_reflectance(state.energy_ior, reflected.micro_cosine) *
                             reflected.factor * state.energy.x,
                         reflected.flags };
        return { kernel::layer_eval(base.response, specular, state.mixture),
                 state.mixture.base_sample_weight + state.mixture.specular_sample_weight > 0
                     ? lerp(base.pdf, reflected.pdf, state.mixture.specular_sample_weight)
                     : 0 };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        if (state.mixture.base_sample_weight + state.mixture.specular_sample_weight <= 0)
            return kernel::empty_proposal();
        if (random.z <= state.mixture.specular_sample_weight &&
            state.mixture.specular_sample_weight > 0) {
            let reflection
                : kernel::ReflectionProposal = Measure.sample(
                      wi, float3(random.xy, random.z / state.mixture.specular_sample_weight),
                      state.geometry);
            var proposal : Proposal = reflection.proposal;
            proposal.response.value *= kernel::specular_reflectance(
                dielectric_fresnel(state.energy_ior), reflection.micro_cosine);
            if (!Measure.has_delta() || kernel::is_non_delta(proposal.response.flags))
                proposal.response.value *= state.energy.x;
            proposal.pdf *= state.mixture.specular_sample_weight;
            return proposal;
        }
        var proposal
            : Proposal =
                  Base.sample(wi,
                              float3(random.xy, (random.z - state.mixture.specular_sample_weight) /
                                                    state.mixture.base_sample_weight),
                              state.base);
        proposal.response =
            kernel::scale_response(proposal.response, state.mixture.transmittance_in);
        proposal.pdf *= state.mixture.base_sample_weight;
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return dielectric_reflectance(state.energy_ior, wi.z);
    }
    public static func delta_pdf(state: State)->float {
        return state.mixture.specular_sample_weight;
    }
    public static func marginal_flags(state: State)->bool {
        return false;
    }
    public static func marginal_required(state: State)->bool {
        return true;
    }
};
}
}
