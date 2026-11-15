// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/microfacet_distribution.slang";

import "bsdf/openpbr/math/microfacet_energy.slang";
import "bsdf/openpbr/math/microfacet_measure.slang";
import "bsdf/openpbr/contract/flags.slang";
import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";

// Exact production-reachable microfacet subset. Generic full-state entry points
// outside the compact opaque and transmission topologies are intentionally absent.

public func primeFullOpenPbrMicrofacetLambda(microfacet: PrimeFullOpenPbrMicrofacet, value: float3)
    ->float
{
    return (length(float3(microfacet.alpha * value.xy, value.z)) / abs(value.z) - sign(value.z)) *
           0.5;
}

public func primeFullOpenPbrMicrofacetD<let Isotropic : bool = false>(
    microfacet: PrimeFullOpenPbrMicrofacet, halfVector: float3)
    ->float
{
    if (Isotropic)
    {
        let alpha2 = microfacet.alpha.x * microfacet.alpha.x;
        let d = dot(halfVector.xy, halfVector.xy) + alpha2 * halfVector.z * halfVector.z;
        let value = alpha2 / (PRIME_FULL_OPENPBR_PI * d * d);
        return value * halfVector.z > 1.0e-20 ? value : 0.0;
    }
    let denominator
        : float =
              primeFullOpenPbrReduceSum(primeFullOpenPbrSquare(halfVector.xy / microfacet.alpha)) +
              primeFullOpenPbrSquare(halfVector.z);
    let result : float = 1.0 / (PRIME_FULL_OPENPBR_PI * microfacet.alpha.x * microfacet.alpha.y *
                                primeFullOpenPbrSquare(denominator));
    return result * halfVector.z > 1.0e-20 ? result : 0.0;
}

/// G2/(4|cos_i|) 的约分形式；避免掠射时先产生无穷 lambda 再计算 0/0。
public func primeFullOpenPbrReflectionFactor(microfacet: PrimeFullOpenPbrMicrofacet, wi: float3,
                                             wo: float3)
    ->float
{
    let incident : float = abs(wi.z);
    let outgoing : float = abs(wo.z);
    let projectedIncident : float = length(float3(microfacet.alpha * wi.xy, wi.z));
    let projectedOutgoing : float = length(float3(microfacet.alpha * wo.xy, wo.z));
    let denominator : float = incident * projectedOutgoing + outgoing * projectedIncident;
    return denominator > 0 ? 0.5 * outgoing / denominator : 0;
}

func primeFullOpenPbrStirlingCorrection(value: float)->float
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

func primeFullOpenPbrLogGamma(value: float)->float
{
    return 0.5 * log(2.0 * PRIME_FULL_OPENPBR_PI) - value + (value - 0.5) * log(value) +
           primeFullOpenPbrStirlingCorrection(value);
}

func primeFullOpenPbrGamma(value: float)->float
{
    return exp(primeFullOpenPbrLogGamma(value + 5.0)) /
           (value * (value + 1.0) * (value + 2.0) * (value + 3.0) * (value + 4.0));
}

func primeFullOpenPbrBeta(first: float, second: float)->float
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
        return exp(primeFullOpenPbrLogGamma(a + 5) + primeFullOpenPbrLogGamma(b + 5) -
                   primeFullOpenPbrLogGamma(sum + 5)) *
               (ps / (pa * pb));
    }
    // log Γ(b)-log Γ(a+b) 先代数约分，既不构造 Γ，也不相减两个巨大的 log Γ。
    let r : float = a / b;
    let log1p : float = r < 0.001 ? r * (1 - r * (0.5 - r * (1.0 / 3.0))) : log(1 + r);
    let logGammaA : float = primeFullOpenPbrLogGamma(a + 5) -
                            (log(a) + log(a + 1) + log(a + 2) + log(a + 3) + log(a + 4));
    return exp(logGammaA - a * log(b) - (a + b - 0.5) * log1p + a +
               primeFullOpenPbrStirlingCorrection(b) - primeFullOpenPbrStirlingCorrection(a + b));
}

public func primeFullOpenPbrMicrofacetG2t(microfacet: PrimeFullOpenPbrMicrofacet, wi: float3,
                                          wo: float3)
    ->float
{
    var wi = wi;
    var wo = wo;
    if (wi.z < 0.0)
    {
        wi = -wi;
        wo = -wo;
    }
    return primeFullOpenPbrBeta(primeFullOpenPbrMicrofacetLambda(microfacet, -wi),
                                primeFullOpenPbrMicrofacetLambda(microfacet, wo));
}

public func primeFullOpenPbrMicrofacetSample(microfacet: PrimeFullOpenPbrMicrofacet, wi: float3,
                                             sampleValue: float2, reflectOnly: bool)
    ->float3
{
    let visibleHalf : float3 = normalize(float3(microfacet.alpha * wi.xy, wi.z));
    let phi : float = 2.0 * PRIME_FULL_OPENPBR_PI * sampleValue.x;
    var k = 1.0;
    if (reflectOnly)
    {
        let a : float = clamp(min(microfacet.alpha.x, microfacet.alpha.y), 0.0, 1.0);
        let s : float = 1.0 + length(wi.xy);
        let a2 : float = a * a;
        let s2 : float = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    let z : float = lerp(1.0, -k * visibleHalf.z, sampleValue.y);
    let sine : float = sqrt(clamp(1.0 - z * z, 0.0, 1.0));
    let halfVector : float3 = float3(sine * cos(phi), sine * sin(phi), z) + visibleHalf;
    return normalize(float3(microfacet.alpha * halfVector.xy, max(0.0, halfVector.z)));
}

public func primeFullOpenPbrMicrofacetPdf(microfacet: PrimeFullOpenPbrMicrofacet, wi: float3,
                                          halfVector: float3, reflectOnly: bool)
    ->float
{
    return primeFullOpenPbrMicrofacetPdfFromDistribution(
        microfacet, wi, halfVector, reflectOnly,
        primeFullOpenPbrMicrofacetD(microfacet, halfVector));
}

/// 已有 D 时保留原 PDF 算序，避免同方向求值再次计算分布。
public func primeFullOpenPbrMicrofacetPdfFromDistribution(microfacet: PrimeFullOpenPbrMicrofacet,
                                                          wi: float3, halfVector: float3,
                                                          reflectOnly: bool, distribution: float)
    ->float
{
    let lengthV : float = length(float3(microfacet.alpha * wi.xy, wi.z));
    var k = 1.0;
    if (reflectOnly)
    {
        let a : float = clamp(min(microfacet.alpha.x, microfacet.alpha.y), 0.0, 1.0);
        let s : float = 1.0 + length(wi.xy);
        let a2 : float = a * a;
        let s2 : float = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    return 2.0 * distribution * dot(wi, halfVector) / (k * wi.z + lengthV);
}

/// 反射方向 PDF；消去微法线到方向的 Jacobian 中共同的 dot(wi,h)，包括其零极限。
public func primeFullOpenPbrReflectionPdf(microfacet: PrimeFullOpenPbrMicrofacet, wi: float3,
                                          reflectOnly: bool, distribution: float)
    ->float
{
    var k : float = 1;
    if (reflectOnly)
    {
        let a : float = min(microfacet.alpha.x, microfacet.alpha.y);
        let s : float = 1 + length(wi.xy);
        let a2 : float = a * a;
        let s2 : float = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    return distribution / (2 * (k * wi.z + length(float3(microfacet.alpha * wi.xy, wi.z))));
}

public struct PrimeFullOpenPbrReflectiveEvalBase
{
    public var factor : float;
    public var microCosine : float;
    public var flags : uint;
    public var pdf : float;
};

public struct PrimeFullOpenPbrReflectiveSampleBase
{
    public var bsdfSample : PrimeFullOpenPbrSample;
    public var microCosine : float;
};

public func primeFullOpenPbrReflectiveEvalBase(wi: float3, wo: float3,
                                               microfacet: PrimeFullOpenPbrMicrofacet,
                                               samplingFlags: uint, checkFlags: uint)
    ->PrimeFullOpenPbrReflectiveEvalBase
{
    return primeFullOpenPbrReflectiveEvalBaseImpl<false>(wi, wo, microfacet, samplingFlags,
                                                         checkFlags);
}

public func primeFullOpenPbrReflectiveEvalBaseImpl<
    let Continuous : bool, let Isotropic : bool = false>(wi: float3, wo: float3,
                                                         microfacet: PrimeFullOpenPbrMicrofacet,
                                                         samplingFlags: uint, checkFlags: uint)
    ->PrimeFullOpenPbrReflectiveEvalBase
{
    var result : PrimeFullOpenPbrReflectiveEvalBase;
    result.factor = 0.0;
    result.microCosine = 0.0;
    result.flags = PRIME_FULL_OPENPBR_FLAG_NONE;
    result.pdf = 0.0;
    if ((!Continuous && primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet)) || wi.z < 0.0 ||
        wo.z <= 0.0 || (samplingFlags & (checkFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR)) == 0u)
    {
        return result;
    }
    var halfVector : float3 = wi + wo;
    let lengthSquared : float = dot(halfVector, halfVector);
    if (lengthSquared == 0.0)
    {
        return result;
    }
    halfVector *= rsqrt(lengthSquared);
    result.microCosine = dot(wi, halfVector);
    let distribution : float = primeFullOpenPbrMicrofacetD<Isotropic>(microfacet, halfVector);
    result.factor = distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
    result.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    result.pdf = primeFullOpenPbrReflectionPdf(microfacet, wi, true, distribution);
    return result;
}

public func primeFullOpenPbrReflectiveSampleBase(wi: float3, randomValue: float3,
                                                 microfacet: PrimeFullOpenPbrMicrofacet,
                                                 samplingFlags: uint, checkFlags: uint)
    ->PrimeFullOpenPbrReflectiveSampleBase
{
    return primeFullOpenPbrReflectiveSampleBaseImpl<false>(wi, randomValue, microfacet,
                                                           samplingFlags, checkFlags);
}

public func primeFullOpenPbrReflectiveSampleBaseImpl<
    let Continuous : bool, let Isotropic : bool = false>(wi: float3, randomValue: float3,
                                                         microfacet: PrimeFullOpenPbrMicrofacet,
                                                         samplingFlags: uint, checkFlags: uint)
    ->PrimeFullOpenPbrReflectiveSampleBase
{
    var result : PrimeFullOpenPbrReflectiveSampleBase;
    result.bsdfSample = primeFullOpenPbrZeroSample();
    result.microCosine = 0.0;
    if (wi.z < 0.0)
    {
        return result;
    }
    var halfVector = float3(0.0, 0.0, 1.0);
    var distribution = 1.0;
    if (Continuous || !primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet))
    {
        if ((samplingFlags & (checkFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR)) == 0u)
        {
            return result;
        }
        halfVector = primeFullOpenPbrMicrofacetSample(microfacet, wi, randomValue.xy, true);
        distribution = primeFullOpenPbrMicrofacetD<Isotropic>(microfacet, halfVector);
    }
    else if ((samplingFlags & (checkFlags & PRIME_FULL_OPENPBR_FLAG_DELTA)) == 0u)
    {
        return result;
    }
    var wo : float3 = reflect(-wi, halfVector);
    if (wo.z <= 0.0)
    {
        return result;
    }
    result.microCosine = dot(halfVector, wi);
    result.bsdfSample.wo = wo;
    if (!Continuous && primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet))
    {
        result.bsdfSample.throughput.value = float3(1.0);
        result.bsdfSample.throughput.flags = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
        result.bsdfSample.pdf = 1.0;
    }
    else
    {
        result.bsdfSample.throughput.value =
            float3(distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo));
        result.bsdfSample.throughput.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
        result.bsdfSample.pdf = primeFullOpenPbrReflectionPdf(microfacet, wi, true, distribution);
    }
    return result;
}

public func primeFullOpenPbrReflectivePdf(wi: float3, wo: float3,
                                          microfacet: PrimeFullOpenPbrMicrofacet,
                                          samplingFlags: uint, checkFlags: uint)
    ->float
{
    return primeFullOpenPbrReflectivePdfImpl<false>(wi, wo, microfacet, samplingFlags, checkFlags);
}

public func primeFullOpenPbrReflectivePdfImpl<let Continuous : bool>(
    wi: float3, wo: float3, microfacet: PrimeFullOpenPbrMicrofacet, samplingFlags: uint,
    checkFlags: uint)
    ->float
{
    return primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(wi, wo, microfacet, samplingFlags,
                                                              checkFlags)
        .pdf;
}
