/// 首表面提案组合。物理响应复用原闭包；固定概率只作用于界面反射／进入内部的选择。
module primary_composition;
import openpbr.selection;
import openpbr.composition;
import openpbr.primary_policy;
import openpbr.api;
import openpbr.math.microfacet_energy;
import openpbr.contract.flags;
import openpbr.opaque.state;
import openpbr.opaque.lobes;
import openpbr.math.microfacet_distribution;

namespace openpbr {
namespace closure {
public struct PrimaryLayer<Measure : IMeasure, Base : IBase> : IClosure {
    public typedef DielectricLayer<Measure, Base> Closure;
    public typedef Closure.State State;
    public static func has_delta()->bool {
        return Measure.has_delta();
    }
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        var state : State = Closure.prepare(energy, material, wi, incident);
        let available
            : bool = state.mixture.specular_sample_weight + state.mixture.base_sample_weight > 0;
        let q : float = openpbr::primary::reflection_probability(
                    state.mixture.specular_sample_weight, state.mixture.base_sample_weight);
        state.mixture.specular_sample_weight = available ? q : 0;
        state.mixture.base_sample_weight = available ? 1 - q : 0;
        return state;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return Closure.evaluate(wi, wo, state);
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        return Closure.sample(wi, random, state);
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return Closure.delta_response(wi, state);
    }
    public static func delta_pdf(state: State)->float {
        return Closure.delta_pdf(state);
    }
    public static func marginal_flags(state: State)->bool {
        return false;
    }
    public static func marginal_required(state: State)->bool {
        return true;
    }
};

/// 同一微表面分布上的金属／介电反射合并为一个提案。metalness只参与响应，不冒充提案概率。
public struct PrimaryMetalLayer<Measure : IMeasure, Base : IBase> : IClosure {
    public typedef DielectricLayer<Measure, Base> Dielectric;
    public typedef SharedMetalBlend<Measure, Dielectric> Closure;
    public typedef Closure.State State;
    public static func has_delta()->bool {
        return Measure.has_delta();
    }
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        return Closure.prepare(energy, material, wi, incident);
    }
    public static func probability(state: State)->float {
        return openpbr::primary::reflection_probability(
            state.metalness +
                (1 - state.metalness) * state.dielectric.mixture.specular_sample_weight,
            (1 - state.metalness) * state.dielectric.mixture.base_sample_weight);
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        let reflected : kernel::ReflectionEvaluation =
                            Measure.evaluate(wi, wo, state.conductor.geometry);
        let base = Base.evaluate(wi, wo, state.dielectric.base);
        let a : LocalEvaluation = Dielectric.from_base(base, state.dielectric, reflected);
        let b : LocalEvaluation =
                    Conductor<Measure>.from_reflection(wi, wo, state.conductor, reflected);
        let q : float = probability(state);
        let pdf : float = q * reflected.pdf + (1 - q) * base.pdf;
        return { kernel::mix_eval(a.response, b.response, state.metalness), pdf };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        let q : float = probability(state);
        var proposal : Proposal;
        if (q > 0 && random.z < q) {
            proposal = Measure.sample(wi, float3(random.xy, random.z / q), state.conductor.geometry)
                           .proposal;
            if (Measure.has_delta() && kernel::is_delta(proposal.response.flags)) {
                proposal.response.value = Closure.delta_response(wi, state);
                proposal.pdf *= q;
            }
        } else
            proposal =
                Base.sample(wi, float3(random.xy, (random.z - q) / (1 - q)), state.dielectric.base);
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return Closure.delta_response(wi, state);
    }
    public static func delta_pdf(state: State)->float {
        return probability(state);
    }
    public static func marginal_flags(state: State)->bool {
        return true;
    }
    public static func marginal_required(state: State)->bool {
        return true;
    }
};

public struct PrimaryMetalBase<Measure : IMeasure, Base : IBase> : IClosure {
    public typedef MetalBlend<BaseClosure<Base>, Conductor<Measure>> Closure;
    public typedef Closure.State State;
    public static func has_delta()->bool {
        return Measure.has_delta();
    }
    public static func prepare<EnergyLut : IEnergyLut>(energy: EnergyLut,
                                                       material: openpbr::Material, wi: float3,
                                                       incident: float)
        ->State {
        return Closure.prepare(energy, material, wi, incident);
    }
    public static func probability(wi: float3, state: State)->float {
        let base = Base.energy(wi, state.dielectric);
        return openpbr::primary::reflection_probability(
            state.metalness, (1 - state.metalness) * max(base.x, max(base.y, base.z)));
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        let a : LocalEvaluation = Base.evaluate(wi, wo, state.dielectric);
        let b : LocalEvaluation = Conductor<Measure>.evaluate(wi, wo, state.conductor);
        return { kernel::mix_eval(a.response, b.response, state.metalness),
                 lerp(a.pdf, b.pdf, probability(wi, state)) };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        let q : float = probability(wi, state);
        var proposal : Proposal;
        if (q > 0 && random.z < q) {
            proposal =
                Conductor<Measure>.sample(wi, float3(random.xy, random.z / q), state.conductor);
            if (Measure.has_delta() && kernel::is_delta(proposal.response.flags)) {
                proposal.response.value *= state.metalness;
                proposal.pdf *= q;
            }
        } else
            proposal =
                Base.sample(wi, float3(random.xy, (random.z - q) / (1 - q)), state.dielectric);
        return proposal;
    }
    public static func delta_response(wi: float3, state: State)->float3 {
        return Closure.delta_response(wi, state);
    }
    // 不由采样器消费：此组合直接在离散样本处写入依赖wi的概率。
    public static func delta_pdf(state: State)->float {
        return 0;
    }
    public static func marginal_flags(state: State)->bool {
        return true;
    }
    public static func marginal_required(state: State)->bool {
        return true;
    }
};

static const let primary_has_layer : bool = (selected_flow & FLOW_DIELECTRIC_REFLECTION) != 0 &&
                                            (selected_flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE)) != 0;
static const let primary_has_reflection : bool = (selected_flow & FLOW_DIELECTRIC_REFLECTION) != 0;
static const let primary_has_base_mixture
    : bool = (selected_flow & FLOW_METAL_MIX) != 0 &&
             (selected_flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE)) != 0;
static const let primary_has_metal_mixture : bool = (selected_flow & FLOW_METAL_MIX) != 0;
static const let primary_has_metal : bool = (selected_flow & FLOW_CONDUCTOR_REFLECTION) != 0;
public typealias PrimaryDielectric =
    TypeSelect<primary_has_layer, PrimaryLayer<SelectedMeasure, SelectedBase>, Dielectric>.Type;
typealias PrimaryLayerMixture = PrimaryMetalLayer<SelectedMeasure, SelectedBase>;
typealias PrimaryBaseOnlyMixture = PrimaryMetalBase<SelectedMeasure, SelectedBase>;
typealias PrimaryBaseMixture =
    TypeSelect<primary_has_reflection, PrimaryLayerMixture, PrimaryBaseOnlyMixture>.Type;
typealias PrimaryUnmixed = TypeSelect<primary_has_metal, Metal, PrimaryDielectric>.Type;
typealias PrimaryWithoutBase = TypeSelect<primary_has_metal_mixture, Opaque, PrimaryUnmixed>.Type;
public typealias PrimaryOpaque =
    TypeSelect<primary_has_base_mixture, PrimaryBaseMixture, PrimaryWithoutBase>.Type;
}
}
