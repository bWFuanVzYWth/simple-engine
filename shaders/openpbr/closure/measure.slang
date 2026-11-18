// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 反射的离散、连续与混合测度；只建立所需微面状态。
module measure;
import openpbr.closure.contracts;
import openpbr.contract.flags;
import openpbr.math.spectrum;
import openpbr.math.microfacet_distribution;
import openpbr.math.microfacet_measure;
import openpbr.math.microfacet_energy;
import openpbr.contract.sample;
import openpbr.opaque.state;

namespace openpbr {
namespace closure {
public struct Smooth : IMeasure {
    public static func has_delta()->bool {
        return true;
    }
    public typedef Empty State;
    public static func prepare(roughness: float)->State {
        return {};
    }
    public static func prepare_alpha(alpha: float)->State {
        return {};
    }
    public static func distribution(state: State)->kernel::Microfacet {
        return {};
    }
    public static func delta(state: State)->bool {
        return true;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)
        ->kernel::ReflectionEvaluation {
        return {};
    }
    public static func sample(wi: float3, random: float3, state: State)
        ->kernel::ReflectionProposal {
        return kernel::sample_reflection(wi, random, distribution(state), kernel::LOBE_ALL,
                                         kernel::LOBE_REFLECTION);
    }
    public static func conductor_energy(wi: float3, state: State)->float {
        return 1;
    }
    public static func dielectric_energy<EnergyLut : IEnergyLut>(energy: EnergyLut, wi: float3,
                                                                 ior: float, state: State,
                                                                 roughness: float)
        ->float3 {
        return kernel::dielectric_energy<true, false, false, true>(energy, distribution(state),
                                                                   wi.z, ior, roughness);
    }
};
public struct Continuous : IMeasure {
    public static func has_delta()->bool {
        return false;
    }
    public typedef float State;
    public static func prepare(roughness: float)->State {
        return roughness * roughness;
    }
    public static func prepare_alpha(alpha: float)->State {
        return alpha;
    }
    public static func distribution(state: State)->kernel::Microfacet {
        return { float2(state) };
    }
    public static func delta(state: State)->bool {
        return false;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)
        ->kernel::ReflectionEvaluation {
        return kernel::evaluate_reflection_impl<true, true>(
            wi, wo, distribution(state), kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
    }
    public static func sample(wi: float3, random: float3, state: State)
        ->kernel::ReflectionProposal {
        return kernel::sample_reflection_impl<true, true>(
            wi, random, distribution(state), kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
    }
    public static func conductor_energy(wi: float3, state: State)->float {
        return kernel::microfacet_directional_albedo_reflection<true>(distribution(state), wi.z,
                                                                      float3(1), float3(1))
            .x;
    }
    public static func dielectric_energy<EnergyLut : IEnergyLut>(energy: EnergyLut, wi: float3,
                                                                 ior: float, state: State,
                                                                 roughness: float)
        ->float3 {
        return kernel::dielectric_energy<false, true, false, true>(energy, distribution(state),
                                                                   wi.z, ior, roughness);
    }
};
public struct MixedMeasure : IMeasure {
    public static func has_delta()->bool {
        return true;
    }
    public typedef float State;
    public static func prepare(roughness: float)->State {
        return roughness * roughness;
    }
    public static func prepare_alpha(alpha: float)->State {
        return alpha;
    }
    public static func distribution(state: State)->kernel::Microfacet {
        return { float2(state) };
    }
    public static func delta(state: State)->bool {
        return state < 1.0e-4;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)
        ->kernel::ReflectionEvaluation {
        return kernel::evaluate_reflection_impl<false, true>(
            wi, wo, distribution(state), kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
    }
    public static func sample(wi: float3, random: float3, state: State)
        ->kernel::ReflectionProposal {
        return kernel::sample_reflection_impl<false, true>(
            wi, random, distribution(state), kernel::LOBE_ALL, kernel::LOBE_REFLECTION);
    }
    public static func conductor_energy(wi: float3, state: State)->float {
        return delta(state) ? 1 : Continuous.conductor_energy(wi, state);
    }
    public static func dielectric_energy<EnergyLut : IEnergyLut>(energy: EnergyLut, wi: float3,
                                                                 ior: float, state: State,
                                                                 roughness: float)
        ->float3 {
        return Continuous.dielectric_energy(energy, wi, ior, state, roughness);
    }
};
}
}
