/// 首表面提案组合。物理响应复用原闭包；固定概率只作用于界面反射／进入内部的选择。
module primary_composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.composition;
import bsdf.openpbr.primary_policy;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/math/microfacet_distribution.slang";

namespace openpbr_composition
{
public struct PrimaryLayer<G : IMeasure, B : IBase> : IClosure
{
    public typedef DielectricLayer<G, B> Closure;
    public typedef Closure.State State;
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, material: openpbr::Material, wi: float3, incident: float)
        ->State
    {
        var s : State = Closure.prepare(energy, material, wi, incident);
        let available : bool = s.mixture.specularSampleWeight + s.mixture.baseSampleWeight > 0;
        let q : float = openpbr_primary::reflection_probability(s.mixture.specularSampleWeight,
                                                                s.mixture.baseSampleWeight);
        s.mixture.specularSampleWeight = available ? q : 0;
        s.mixture.baseSampleWeight = available ? 1 - q : 0;
        return s;
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        return Closure.evaluate(wi, wo, s);
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        return Closure.sample(wi, u, s);
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return Closure.delta_response(wi, s);
    }
    public static func delta_pdf(s: State)->float
    {
        return Closure.delta_pdf(s);
    }
    public static func marginal_flags(s: State)->bool
    {
        return false;
    }
    public static func marginal_required(s: State)->bool
    {
        return true;
    }
};

/// 同一微表面分布上的金属／介电反射合并为一个提案。metalness只参与响应，不冒充提案概率。
public struct PrimaryMetalLayer<G : IMeasure, B : IBase> : IClosure
{
    public typedef DielectricLayer<G, B> Dielectric;
    public typedef SharedMetalBlend<G, Dielectric> Closure;
    public typedef Closure.State State;
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, material: openpbr::Material, wi: float3, incident: float)
        ->State
    {
        return Closure.prepare(energy, material, wi, incident);
    }
    public static func probability(s: State)->float
    {
        return openpbr_primary::reflection_probability(
            s.metalness + (1 - s.metalness) * s.dielectric.mixture.specularSampleWeight,
            (1 - s.metalness) * s.dielectric.mixture.baseSampleWeight);
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        let reflected : PrimeFullOpenPbrReflectiveEvalBase =
                            G.evaluate(wi, wo, s.conductor.geometry);
        let a : Value = Dielectric.from_reflection(wi, wo, s.dielectric, reflected);
        let b : Value = Conductor<G>.from_reflection(wi, wo, s.conductor, reflected);
        let q : float = probability(s);
        let pdf : float = q * reflected.pdf + (1 - q) * B.evaluate(wi, wo, s.dielectric.base).pdf;
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.metalness), pdf };
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        let q : float = probability(s);
        var p : Proposal;
        if (q > 0 && u.z < q)
        {
            p = G.sample(wi, float3(u.xy, u.z / q), s.conductor.geometry).bsdfSample;
            if (G.has_delta() && primeFullOpenPbrIsDelta(p.throughput.flags))
            {
                p.throughput.value = Closure.delta_response(wi, s);
                p.pdf *= q;
            }
        }
        else
            p = B.sample(wi, float3(u.xy, (u.z - q) / (1 - q)), s.dielectric.base);
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return Closure.delta_response(wi, s);
    }
    public static func delta_pdf(s: State)->float
    {
        return probability(s);
    }
    public static func marginal_flags(s: State)->bool
    {
        return true;
    }
    public static func marginal_required(s: State)->bool
    {
        return true;
    }
};

public struct PrimaryMetalBase<G : IMeasure, B : IBase> : IClosure
{
    public typedef MetalBlend<BaseClosure<B>, Conductor<G>> Closure;
    public typedef Closure.State State;
    public static func has_delta()->bool
    {
        return G.has_delta();
    }
    public static func prepare<P : IPrimeFullOpenPbrTransmissionGgxEnergy>(
        energy: P, material: openpbr::Material, wi: float3, incident: float)
        ->State
    {
        return Closure.prepare(energy, material, wi, incident);
    }
    public static func probability(wi: float3, s: State)->float
    {
        return openpbr_primary::reflection_probability(
            s.metalness, (1 - s.metalness) *
                             max(B.energy(wi, s.dielectric).x,
                                 max(B.energy(wi, s.dielectric).y, B.energy(wi, s.dielectric).z)));
    }
    public static func evaluate(wi: float3, wo: float3, s: State)->Value
    {
        let a : Value = B.evaluate(wi, wo, s.dielectric);
        let b : Value = Conductor<G>.evaluate(wi, wo, s.conductor);
        return { primeFullCompactMixEval(a.throughput, b.throughput, s.metalness),
                 lerp(a.pdf, b.pdf, probability(wi, s)) };
    }
    public static func sample(wi: float3, u: float3, s: State)->Proposal
    {
        let q : float = probability(wi, s);
        var p : Proposal;
        if (q > 0 && u.z < q)
        {
            p = Conductor<G>.sample(wi, float3(u.xy, u.z / q), s.conductor);
            if (G.has_delta() && primeFullOpenPbrIsDelta(p.throughput.flags))
            {
                p.throughput.value *= s.metalness;
                p.pdf *= q;
            }
        }
        else
            p = B.sample(wi, float3(u.xy, (u.z - q) / (1 - q)), s.dielectric);
        return p;
    }
    public static func delta_response(wi: float3, s: State)->float3
    {
        return Closure.delta_response(wi, s);
    }
    // 不由采样器消费：此组合直接在离散样本处写入依赖wi的概率。
    public static func delta_pdf(s: State)->float
    {
        return 0;
    }
    public static func marginal_flags(s: State)->bool
    {
        return true;
    }
    public static func marginal_required(s: State)->bool
    {
        return true;
    }
};

static const let PrimaryHasLayer : bool = (SelectedFlow & 2) != 0 && (SelectedFlow & 9) != 0;
static const let PrimaryHasReflection : bool = (SelectedFlow & 2) != 0;
static const let PrimaryHasBaseMixture : bool =
                                             (SelectedFlow & 2048) != 0 && (SelectedFlow & 9) != 0;
static const let PrimaryHasMetalMixture : bool = (SelectedFlow & 2048) != 0;
static const let PrimaryHasMetal : bool = (SelectedFlow & 4) != 0;
public typealias PrimaryDielectric =
    TypeSelect<PrimaryHasLayer, PrimaryLayer<SelectedMeasure, SelectedBase>, Dielectric>.Type;
typealias PrimaryLayerMixture = PrimaryMetalLayer<SelectedMeasure, SelectedBase>;
typealias PrimaryBaseOnlyMixture = PrimaryMetalBase<SelectedMeasure, SelectedBase>;
typealias PrimaryBaseMixture =
    TypeSelect<PrimaryHasReflection, PrimaryLayerMixture, PrimaryBaseOnlyMixture>.Type;
typealias PrimaryUnmixed = TypeSelect<PrimaryHasMetal, Metal, PrimaryDielectric>.Type;
typealias PrimaryWithoutBase = TypeSelect<PrimaryHasMetalMixture, Opaque, PrimaryUnmixed>.Type;
public typealias PrimaryOpaque =
    TypeSelect<PrimaryHasBaseMixture, PrimaryBaseMixture, PrimaryWithoutBase>.Type;
}
