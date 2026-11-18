// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 吸收、Lambert与薄层SSS底层；切换与混合保留各自原有随机映射。
module base;
import openpbr.closure.contracts;
import openpbr.contract.flags;
import openpbr.math.spectrum;
import openpbr.opaque.lobes;
import openpbr.subsurface;
import math.constants;
import openpbr.contract.sample;
import openpbr.opaque.state;
import openpbr.math.microfacet_energy;

namespace openpbr {
namespace closure {
public struct Absorber : IBase {
    public static func marginal_required()->bool {
        return false;
    }

    public typedef Empty State;
    public static func prepare(color: float3, weight: float)->State {
        return {};
    }
    public static func energy(wi: float3, state: State)->float3 {
        return 0;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return {};
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        return kernel::empty_proposal();
    }
};
public struct Lambert : IBase {
    public static func marginal_required()->bool {
        return false;
    }

    public typedef float3 State;
    public static func prepare(color: float3, weight: float)->State {
        return color;
    }
    public static func energy(wi: float3, state: State)->float3 {
        return wi.z >= 0 ? state : float3(0);
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        if (wi.z < 0 || wo.z < 0)
            return {};
        let pdf : float = math::frac_1_pi * wo.z;
        return { { state * pdf, kernel::LOBE_DIFFUSE_REFLECTION }, pdf };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        var proposal : Proposal = kernel::empty_proposal();
        if (wi.z < 0)
            return proposal;
        proposal.wo = kernel::cosine_sample_hemisphere(random.xy);
        proposal.pdf = math::frac_1_pi * proposal.wo.z;
        proposal.response = { state * proposal.pdf, kernel::LOBE_DIFFUSE_REFLECTION };
        return proposal;
    }
};
public struct Subsurface : IBase {
    public static func marginal_required()->bool {
        return false;
    }

    public typedef float3 State;
    public static func prepare(color: float3, weight: float)->State {
        return kernel::subsurface_color(color);
    }
    public static func energy(wi: float3, state: State)->float3 {
        return state;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        let reflected : bool = wi.z * wo.z > 0;
        let cosine : float = math::frac_1_pi * abs(wo.z);
        return {
            { float3(cosine) * state * 0.5 * (reflected ? 1.3333333730697632 : 0.6666666269302368),
              reflected ? kernel::LOBE_DIFFUSE_REFLECTION : kernel::LOBE_DIFFUSE_TRANSMISSION },
            cosine * (reflected ? 0.6666666865348816 : 0.3333333134651184)
        };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        var proposal : Proposal = kernel::empty_proposal();
        proposal.wo = kernel::cosine_sample_hemisphere(random.xy);
        proposal.pdf = math::frac_1_pi * proposal.wo.z;
        proposal.response.value = float3(proposal.pdf);
        if (wi.z < 0)
            proposal.wo.z = -proposal.wo.z;
        if (random.z <= 0.3333333134651184) {
            proposal.wo.z = -proposal.wo.z;
            proposal.pdf *= 0.3333333134651184;
            proposal.response.value *= 0.3333333134651184;
            proposal.response.value *= state;
            proposal.response.flags = kernel::LOBE_DIFFUSE_TRANSMISSION;
        } else {
            proposal.pdf *= 0.6666666865348816;
            proposal.response.value *= 0.6666666865348816 * state;
            proposal.response.flags = kernel::LOBE_DIFFUSE_REFLECTION;
        }
        return proposal;
    }
};
public struct BaseSwitchState {
    public var color : float3;
    public var subsurface : bool;
};
public struct SubsurfaceSwitch : IBase {
    public static func marginal_required()->bool {
        return false;
    }

    public typedef BaseSwitchState State;
    public static func prepare(color: float3, weight: float)->State {
        return { weight == 1 ? Subsurface.prepare(color, 1) : color, weight == 1 };
    }
    public static func energy(wi: float3, state: State)->float3 {
        return state.subsurface ? Subsurface.energy(wi, state.color)
                                : Lambert.energy(wi, state.color);
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        return state.subsurface ? Subsurface.evaluate(wi, wo, state.color)
                                : Lambert.evaluate(wi, wo, state.color);
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        return state.subsurface ? Subsurface.sample(wi, random, state.color)
                                : Lambert.sample(wi, random, state.color);
    }
};
public struct BaseBlendState {
    public var diffuse : float3;
    public var subsurface : float3;
    public var weight : float;
    public var proposal : kernel::MixState;
};
public struct SubsurfaceBlend : IBase {
    public static func marginal_required()->bool {
        return true;
    }

    public typedef BaseBlendState State;
    public static func prepare(color: float3, weight: float)->State {
        let sss : float3 = weight > 0 ? Subsurface.prepare(color, 1) : float3(0);
        return { color, sss, weight, kernel::make_mix_state(weight, color, sss) };
    }
    public static func energy(wi: float3, state: State)->float3 {
        return state.diffuse * (1 - state.weight) + state.subsurface * state.weight;
    }
    public static func evaluate(wi: float3, wo: float3, state: State)->LocalEvaluation {
        let a : LocalEvaluation =
                    state.weight < 1 ? Lambert.evaluate(wi, wo, state.diffuse) : LocalEvaluation();
        let b : LocalEvaluation = state.weight > 0 ? Subsurface.evaluate(wi, wo, state.subsurface)
                                                   : LocalEvaluation();
        return { kernel::mix_eval(a.response, b.response, state.weight),
                 state.proposal.first_sample_weight + state.proposal.second_sample_weight > 0
                     ? lerp(a.pdf, b.pdf, state.proposal.second_sample_weight)
                     : 0 };
    }
    public static func sample(wi: float3, random: float3, state: State)->Proposal {
        if (state.weight == 0)
            return Lambert.sample(wi, random, state.diffuse);
        if (state.weight == 1)
            return Subsurface.sample(wi, random, state.subsurface);
        if (state.proposal.first_sample_weight + state.proposal.second_sample_weight <= 0)
            return kernel::empty_proposal();
        if (random.z <= state.proposal.second_sample_weight &&
            state.proposal.second_sample_weight > 0) {
            var proposal
                : Proposal = Subsurface.sample(
                      wi, float3(random.xy, random.z / state.proposal.second_sample_weight),
                      state.subsurface);
            proposal.response.value *= state.weight;
            proposal.pdf *= state.proposal.second_sample_weight;
            return proposal;
        }
        // Fractional SSS 保留参考 EON 的随机映射，包括零概率端点。
        let z : float = (random.z - state.proposal.second_sample_weight) /
                        state.proposal.first_sample_weight;
        var proposal : Proposal = kernel::empty_proposal();
        if (wi.z < 0)
            return proposal;
        if (z <= 0) {
            proposal.wo = kernel::uniform_sample_hemisphere(random.xy);
            proposal.pdf = proposal.wo.z * math::frac_1_pi;
        } else {
            let radius : float = sqrt(random.x);
            let phi : float = math::tau * random.y;
            let x : float = -radius * cos(phi);
            let y : float = radius * sin(phi);
            var wo = float3(x, y, sqrt(max(1 - (x * x + y * y), 0)));
            let n : float = length(wo);
            proposal.pdf = wo.z * math::frac_1_pi * n * n * n;
            proposal.wo = normalize(mul(wo, kernel::eon_basis(wi)));
        }
        proposal.response = Lambert.evaluate(wi, proposal.wo, state.diffuse).response;
        proposal.response.value *= 1 - state.weight;
        proposal.pdf *= state.proposal.first_sample_weight;
        return proposal;
    }
};

/// 测度部件隔离纯delta、纯连续和跨阈值输入；仅跨阈值部件执行数值分类。
}
}
