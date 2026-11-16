// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/microfacet_distribution.slang";

import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.math.microfacet_measure;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.math.spectrum;

import math.constants;
// Exact production-reachable microfacet subset. Generic full-state entry points
// outside the compact opaque and transmission topologies are intentionally absent.

namespace openpbr
{
namespace kernel
{
public func ggx_lambda(microfacet: Microfacet, value: float3)->float
{
    return (length(float3(microfacet.alpha * value.xy, value.z)) / abs(value.z) - sign(value.z)) *
           0.5;
}

public func ggx_distribution<let isotropic : bool = false>(microfacet: Microfacet,
                                                           half_vector: float3)
    ->float
{
    if (isotropic)
    {
        let alpha2 = microfacet.alpha.x * microfacet.alpha.x;
        let d = dot(half_vector.xy, half_vector.xy) + alpha2 * half_vector.z * half_vector.z;
        let value = alpha2 / (math::pi * d * d);
        return value * half_vector.z > 1.0e-20 ? value : 0.0;
    }
    let denominator
        : float = reduce_sum(square(half_vector.xy / microfacet.alpha)) + square(half_vector.z);
    let result
        : float = 1.0 / (math::pi * microfacet.alpha.x * microfacet.alpha.y * square(denominator));
    return result * half_vector.z > 1.0e-20 ? result : 0.0;
}

/// G2/(4|cos_i|) 的约分形式；避免掠射时先产生无穷 lambda 再计算 0/0。
public func reflection_factor(microfacet: Microfacet, wi: float3, wo: float3)->float
{
    let incident : float = abs(wi.z);
    let outgoing : float = abs(wo.z);
    let projected_incident : float = length(float3(microfacet.alpha * wi.xy, wi.z));
    let projected_outgoing : float = length(float3(microfacet.alpha * wo.xy, wo.z));
    let denominator : float = incident * projected_outgoing + outgoing * projected_incident;
    return denominator > 0 ? 0.5 * outgoing / denominator : 0;
}

func stirling_correction(value: float)->float
{
    // 原连分式的同一有理函数；生产 value>=6，正系数分母不会相消。
    let inverse = 1.0 / value;
    let t = inverse * inverse;
    let numerator =
        ((0.5793759231628358 * t + 5.497138891779732) * t + 5.576473112868737) * t + 1.0;
    let denominator =
        ((0.7222456732620475 * t + 5.674608630462658) * t + 5.60980644620207) * t + 1.0;
    return (1.0 / 12.0) * inverse * numerator / denominator;
}

func log_gamma(value: float)->float
{
    return 0.5 * log(math::tau) - value + (value - 0.5) * log(value) + stirling_correction(value);
}

func beta(first: float, second: float)->float
{
    let a : float = min(first, second);
    let b : float = max(first, second);
    if (a == 1)
        return 1 / b;
    if (b < 8)
    {
        let sum = a + b;
        let pa = a * (a + 1) * (a + 2) * (a + 3) * (a + 4);
        let pb = b * (b + 1) * (b + 2) * (b + 3) * (b + 4);
        let ps = sum * (sum + 1) * (sum + 2) * (sum + 3) * (sum + 4);
        return exp(log_gamma(a + 5) + log_gamma(b + 5) - log_gamma(sum + 5)) * (ps / (pa * pb));
    }
    // log Γ(b)-log Γ(a+b) 先代数约分，既不构造 Γ，也不相减两个巨大的 log Γ。
    let r : float = a / b;
    let log1p : float = r < 0.001 ? r * (1 - r * (0.5 - r * (1.0 / 3.0))) : log(1 + r);
    let log_gamma_a
        : float = log_gamma(a + 5) - (log(a) + log(a + 1) + log(a + 2) + log(a + 3) + log(a + 4));
    return exp(log_gamma_a - a * log(b) - (a + b - 0.5) * log1p + a + stirling_correction(b) -
               stirling_correction(a + b));
}

public func ggx_transmission_masking(microfacet: Microfacet, wi: float3, wo: float3)->float
{
    var wi = wi;
    var wo = wo;
    if (wi.z < 0.0)
    {
        wi = -wi;
        wo = -wo;
    }
    return beta(ggx_lambda(microfacet, -wi), ggx_lambda(microfacet, wo));
}

public func sample_ggx_vndf(microfacet: Microfacet, wi: float3, sample_value: float2,
                            reflect_only: bool)
    ->float3
{
    let visible_half : float3 = normalize(float3(microfacet.alpha * wi.xy, wi.z));
    let phi : float = math::tau * sample_value.x;
    var k = 1.0;
    if (reflect_only)
    {
        let a : float = clamp(min(microfacet.alpha.x, microfacet.alpha.y), 0.0, 1.0);
        let s : float = 1.0 + length(wi.xy);
        let a2 : float = a * a;
        let s2 : float = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    let z : float = lerp(1.0, -k * visible_half.z, sample_value.y);
    let sine : float = sqrt(clamp(1.0 - z * z, 0.0, 1.0));
    let half_vector : float3 = float3(sine * cos(phi), sine * sin(phi), z) + visible_half;
    return normalize(float3(microfacet.alpha * half_vector.xy, max(0.0, half_vector.z)));
}

public func ggx_pdf(microfacet: Microfacet, wi: float3, half_vector: float3, reflect_only: bool)
    ->float
{
    return ggx_pdf_from_distribution(microfacet, wi, half_vector, reflect_only,
                                     ggx_distribution(microfacet, half_vector));
}

/// 已有 D 时保留原 PDF 算序，避免同方向求值再次计算分布。
public func ggx_pdf_from_distribution(microfacet: Microfacet, wi: float3, half_vector: float3,
                                      reflect_only: bool, distribution: float)
    ->float
{
    let length_v : float = length(float3(microfacet.alpha * wi.xy, wi.z));
    var k = 1.0;
    if (reflect_only)
    {
        let a : float = clamp(min(microfacet.alpha.x, microfacet.alpha.y), 0.0, 1.0);
        let s : float = 1.0 + length(wi.xy);
        let a2 : float = a * a;
        let s2 : float = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    return 2.0 * distribution * dot(wi, half_vector) / (k * wi.z + length_v);
}

/// 反射方向 PDF；消去微法线到方向的 Jacobian 中共同的 dot(wi,h)，包括其零极限。
public func ggx_reflection_pdf(microfacet: Microfacet, wi: float3, reflect_only: bool,
                               distribution: float)
    ->float
{
    var k : float = 1;
    if (reflect_only)
    {
        let a : float = min(microfacet.alpha.x, microfacet.alpha.y);
        let s : float = 1 + length(wi.xy);
        let a2 : float = a * a;
        let s2 : float = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    return distribution / (2 * (k * wi.z + length(float3(microfacet.alpha * wi.xy, wi.z))));
}

public struct ReflectionEvaluation
{
    public var factor : float;
    public var micro_cosine : float;
    public var flags : uint;
    public var pdf : float;
};

public struct ReflectionProposal
{
    public var proposal : Proposal;
    public var micro_cosine : float;
};

public func evaluate_reflection(wi: float3, wo: float3, microfacet: Microfacet,
                                sampling_flags: uint, check_flags: uint)
    ->ReflectionEvaluation
{
    return evaluate_reflection_impl<false>(wi, wo, microfacet, sampling_flags, check_flags);
}

public func evaluate_reflection_impl<let continuous : bool, let isotropic : bool = false>(
    wi: float3, wo: float3, microfacet: Microfacet, sampling_flags: uint, check_flags: uint)
    ->ReflectionEvaluation
{
    var result : ReflectionEvaluation;
    result.factor = 0.0;
    result.micro_cosine = 0.0;
    result.flags = LOBE_NONE;
    result.pdf = 0.0;
    if ((!continuous && microfacet_effectively_smooth(microfacet)) || wi.z < 0.0 || wo.z <= 0.0 ||
        (sampling_flags & (check_flags & LOBE_SPECULAR)) == 0u)
    {
        return result;
    }
    var half_vector : float3 = wi + wo;
    let length_squared : float = dot(half_vector, half_vector);
    if (length_squared == 0.0)
    {
        return result;
    }
    half_vector *= rsqrt(length_squared);
    result.micro_cosine = dot(wi, half_vector);
    let distribution : float = ggx_distribution<isotropic>(microfacet, half_vector);
    result.factor = distribution * reflection_factor(microfacet, wi, wo);
    result.flags = LOBE_SPECULAR_REFLECTION;
    result.pdf = ggx_reflection_pdf(microfacet, wi, true, distribution);
    return result;
}

public func sample_reflection(wi: float3, random_value: float3, microfacet: Microfacet,
                              sampling_flags: uint, check_flags: uint)
    ->ReflectionProposal
{
    return sample_reflection_impl<false>(wi, random_value, microfacet, sampling_flags, check_flags);
}

public func sample_reflection_impl<let continuous : bool, let isotropic : bool = false>(
    wi: float3, random_value: float3, microfacet: Microfacet, sampling_flags: uint,
    check_flags: uint)
    ->ReflectionProposal
{
    var result : ReflectionProposal;
    result.proposal = empty_proposal();
    result.micro_cosine = 0.0;
    if (wi.z < 0.0)
    {
        return result;
    }
    var half_vector = float3(0.0, 0.0, 1.0);
    var distribution = 1.0;
    if (continuous || !microfacet_effectively_smooth(microfacet))
    {
        if ((sampling_flags & (check_flags & LOBE_SPECULAR)) == 0u)
        {
            return result;
        }
        half_vector = sample_ggx_vndf(microfacet, wi, random_value.xy, true);
        distribution = ggx_distribution<isotropic>(microfacet, half_vector);
    }
    else if ((sampling_flags & (check_flags & LOBE_DELTA)) == 0u)
    {
        return result;
    }
    var wo : float3 = reflect(-wi, half_vector);
    if (wo.z <= 0.0)
    {
        return result;
    }
    result.micro_cosine = dot(half_vector, wi);
    result.proposal.wo = wo;
    if (!continuous && microfacet_effectively_smooth(microfacet))
    {
        result.proposal.response.value = float3(1.0);
        result.proposal.response.flags = LOBE_DELTA_REFLECTION;
        result.proposal.pdf = 1.0;
    }
    else
    {
        result.proposal.response.value =
            float3(distribution * reflection_factor(microfacet, wi, wo));
        result.proposal.response.flags = LOBE_SPECULAR_REFLECTION;
        result.proposal.pdf = ggx_reflection_pdf(microfacet, wi, true, distribution);
    }
    return result;
}

public func reflection_pdf(wi: float3, wo: float3, microfacet: Microfacet, sampling_flags: uint,
                           check_flags: uint)
    ->float
{
    return reflection_pdf_impl<false>(wi, wo, microfacet, sampling_flags, check_flags);
}

public func reflection_pdf_impl<let continuous : bool>(wi: float3, wo: float3,
                                                       microfacet: Microfacet, sampling_flags: uint,
                                                       check_flags: uint)
    ->float
{
    return evaluate_reflection_impl<continuous>(wi, wo, microfacet, sampling_flags, check_flags)
        .pdf;
}
}
}
