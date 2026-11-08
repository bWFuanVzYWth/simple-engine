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

public float primeFullOpenPbrMicrofacetLambda(PrimeFullOpenPbrMicrofacet microfacet, float3 value)
{
    return (sqrt(1.0 +
                 primeFullOpenPbrReduceSum(primeFullOpenPbrSquare(microfacet.alpha * value.xy)) /
                     primeFullOpenPbrSquare(value.z)) -
            sign(value.z)) *
           0.5;
}

public float primeFullOpenPbrMicrofacetD(PrimeFullOpenPbrMicrofacet microfacet, float3 halfVector)
{
    float denominator =
        primeFullOpenPbrReduceSum(primeFullOpenPbrSquare(halfVector.xy / microfacet.alpha)) +
        primeFullOpenPbrSquare(halfVector.z);
    float result = 1.0 / (PRIME_FULL_OPENPBR_PI * microfacet.alpha.x * microfacet.alpha.y *
                          primeFullOpenPbrSquare(denominator));
    return result * halfVector.z > 1.0e-20 ? result : 0.0;
}

/// G2/(4|cos_i|) 的约分形式；避免掠射时先产生无穷 lambda 再计算 0/0。
public float primeFullOpenPbrReflectionFactor(PrimeFullOpenPbrMicrofacet microfacet, float3 wi,
                                              float3 wo)
{
    float incident = abs(wi.z);
    float outgoing = abs(wo.z);
    float projectedIncident = length(float3(microfacet.alpha * wi.xy, wi.z));
    float projectedOutgoing = length(float3(microfacet.alpha * wo.xy, wo.z));
    float denominator = incident * projectedOutgoing + outgoing * projectedIncident;
    return denominator > 0 ? 0.5 * outgoing / denominator : 0;
}

float primeFullOpenPbrStirlingCorrection(float value)
{
    const float gam0 = 1.0 / 12.0;
    const float gam1 = 1.0 / 30.0;
    const float gam2 = 53.0 / 210.0;
    const float gam3 = 195.0 / 371.0;
    const float gam4 = 22999.0 / 22737.0;
    const float gam5 = 29944523.0 / 19733142.0;
    const float gam6 = 109535241009.0 / 48264275462.0;
    return gam0 /
           (value +
            gam1 /
                (value +
                 gam2 / (value + gam3 / (value + gam4 / (value + gam5 / (value + gam6 / value))))));
}

float primeFullOpenPbrLogGamma(float value)
{
    return 0.5 * log(2.0 * PRIME_FULL_OPENPBR_PI) - value + (value - 0.5) * log(value) +
           primeFullOpenPbrStirlingCorrection(value);
}

float primeFullOpenPbrGamma(float value)
{
    return exp(primeFullOpenPbrLogGamma(value + 5.0)) /
           (value * (value + 1.0) * (value + 2.0) * (value + 3.0) * (value + 4.0));
}

float primeFullOpenPbrBeta(float first, float second)
{
    float a = min(first, second);
    float b = max(first, second);
    if (a == 1)
        return 1 / b;
    if (b < 8)
        return primeFullOpenPbrGamma(a) * primeFullOpenPbrGamma(b) / primeFullOpenPbrGamma(a + b);
    // log Γ(b)-log Γ(a+b) 先代数约分，既不构造 Γ，也不相减两个巨大的 log Γ。
    float r = a / b;
    float log1p = r < 0.001 ? r * (1 - r * (0.5 - r / 3)) : log(1 + r);
    float logGammaA = primeFullOpenPbrLogGamma(a + 5) -
                      (log(a) + log(a + 1) + log(a + 2) + log(a + 3) + log(a + 4));
    return exp(logGammaA - a * log(b) - (a + b - 0.5) * log1p + a +
               primeFullOpenPbrStirlingCorrection(b) - primeFullOpenPbrStirlingCorrection(a + b));
}

public float primeFullOpenPbrMicrofacetG2t(PrimeFullOpenPbrMicrofacet microfacet, float3 wi,
                                           float3 wo)
{
    if (wi.z < 0.0)
    {
        wi = -wi;
        wo = -wo;
    }
    return primeFullOpenPbrBeta(primeFullOpenPbrMicrofacetLambda(microfacet, -wi),
                                primeFullOpenPbrMicrofacetLambda(microfacet, wo));
}

public float3 primeFullOpenPbrMicrofacetSample(PrimeFullOpenPbrMicrofacet microfacet, float3 wi,
                                               float2 sampleValue, bool reflectOnly)
{
    float3 visibleHalf = normalize(float3(microfacet.alpha * wi.xy, wi.z));
    float phi = 2.0 * PRIME_FULL_OPENPBR_PI * sampleValue.x;
    float k = 1.0;
    if (reflectOnly)
    {
        float a = clamp(min(microfacet.alpha.x, microfacet.alpha.y), 0.0, 1.0);
        float s = 1.0 + length(wi.xy);
        float a2 = a * a;
        float s2 = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    float z = lerp(1.0, -k * visibleHalf.z, sampleValue.y);
    float sine = sqrt(clamp(1.0 - z * z, 0.0, 1.0));
    float3 halfVector = float3(sine * cos(phi), sine * sin(phi), z) + visibleHalf;
    return normalize(float3(microfacet.alpha * halfVector.xy, max(0.0, halfVector.z)));
}

public float primeFullOpenPbrMicrofacetPdf(PrimeFullOpenPbrMicrofacet microfacet, float3 wi,
                                           float3 halfVector, bool reflectOnly)
{
    return primeFullOpenPbrMicrofacetPdfFromDistribution(
        microfacet, wi, halfVector, reflectOnly,
        primeFullOpenPbrMicrofacetD(microfacet, halfVector));
}

/// 已有 D 时保留原 PDF 算序，避免同方向求值再次计算分布。
public float primeFullOpenPbrMicrofacetPdfFromDistribution(PrimeFullOpenPbrMicrofacet microfacet,
                                                           float3 wi, float3 halfVector,
                                                           bool reflectOnly, float distribution)
{
    float lengthV = length(float3(microfacet.alpha * wi.xy, wi.z));
    float k = 1.0;
    if (reflectOnly)
    {
        float a = clamp(min(microfacet.alpha.x, microfacet.alpha.y), 0.0, 1.0);
        float s = 1.0 + length(wi.xy);
        float a2 = a * a;
        float s2 = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    return 2.0 * distribution * dot(wi, halfVector) / (k * wi.z + lengthV);
}

/// 反射方向 PDF；消去微法线到方向的 Jacobian 中共同的 dot(wi,h)，包括其零极限。
public float primeFullOpenPbrReflectionPdf(PrimeFullOpenPbrMicrofacet microfacet, float3 wi,
                                           bool reflectOnly, float distribution)
{
    float k = 1;
    if (reflectOnly)
    {
        float a = min(microfacet.alpha.x, microfacet.alpha.y);
        float s = 1 + length(wi.xy);
        float a2 = a * a;
        float s2 = s * s;
        k = (s2 - a2 * s2) / (s2 + a2 * wi.z * wi.z);
    }
    return distribution / (2 * (k * wi.z + length(float3(microfacet.alpha * wi.xy, wi.z))));
}

public struct PrimeFullOpenPbrReflectiveEvalBase
{
    public float factor;
    public float microCosine;
    public uint flags;
    public float pdf;
};

public struct PrimeFullOpenPbrReflectiveSampleBase
{
    public PrimeFullOpenPbrSample bsdfSample;
    public float microCosine;
};

public PrimeFullOpenPbrReflectiveEvalBase primeFullOpenPbrReflectiveEvalBase(
    float3 wi, float3 wo, PrimeFullOpenPbrMicrofacet microfacet, uint samplingFlags,
    uint checkFlags)
{
    return primeFullOpenPbrReflectiveEvalBaseImpl<false>(wi, wo, microfacet, samplingFlags,
                                                         checkFlags);
}

public PrimeFullOpenPbrReflectiveEvalBase primeFullOpenPbrReflectiveEvalBaseImpl<
    let Continuous : bool>(float3 wi, float3 wo, PrimeFullOpenPbrMicrofacet microfacet,
                           uint samplingFlags, uint checkFlags)
{
    PrimeFullOpenPbrReflectiveEvalBase result;
    result.factor = 0.0;
    result.microCosine = 0.0;
    result.flags = PRIME_FULL_OPENPBR_FLAG_NONE;
    result.pdf = 0.0;
    if ((!Continuous && primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet)) || wi.z < 0.0 ||
        wo.z <= 0.0 || (samplingFlags & (checkFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR)) == 0u)
    {
        return result;
    }
    float3 halfVector = wi + wo;
    float lengthSquared = dot(halfVector, halfVector);
    if (lengthSquared == 0.0)
    {
        return result;
    }
    halfVector *= rsqrt(lengthSquared);
    result.microCosine = dot(wi, halfVector);
    float distribution = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    result.factor = distribution * primeFullOpenPbrReflectionFactor(microfacet, wi, wo);
    result.flags = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION;
    result.pdf = primeFullOpenPbrReflectionPdf(microfacet, wi, true, distribution);
    return result;
}

public PrimeFullOpenPbrReflectiveSampleBase primeFullOpenPbrReflectiveSampleBase(
    float3 wi, float3 randomValue, PrimeFullOpenPbrMicrofacet microfacet, uint samplingFlags,
    uint checkFlags)
{
    return primeFullOpenPbrReflectiveSampleBaseImpl<false>(wi, randomValue, microfacet,
                                                           samplingFlags, checkFlags);
}

public PrimeFullOpenPbrReflectiveSampleBase primeFullOpenPbrReflectiveSampleBaseImpl<
    let Continuous : bool>(float3 wi, float3 randomValue, PrimeFullOpenPbrMicrofacet microfacet,
                           uint samplingFlags, uint checkFlags)
{
    PrimeFullOpenPbrReflectiveSampleBase result;
    result.bsdfSample = primeFullOpenPbrZeroSample();
    result.microCosine = 0.0;
    if (wi.z < 0.0)
    {
        return result;
    }
    float3 halfVector = float3(0.0, 0.0, 1.0);
    float distribution = 1.0;
    if (Continuous || !primeFullOpenPbrMicrofacetEffectivelySmooth(microfacet))
    {
        if ((samplingFlags & (checkFlags & PRIME_FULL_OPENPBR_FLAG_SPECULAR)) == 0u)
        {
            return result;
        }
        halfVector = primeFullOpenPbrMicrofacetSample(microfacet, wi, randomValue.xy, true);
        distribution = primeFullOpenPbrMicrofacetD(microfacet, halfVector);
    }
    else if ((samplingFlags & (checkFlags & PRIME_FULL_OPENPBR_FLAG_DELTA)) == 0u)
    {
        return result;
    }
    float3 wo = reflect(-wi, halfVector);
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

public float primeFullOpenPbrReflectivePdf(float3 wi, float3 wo,
                                           PrimeFullOpenPbrMicrofacet microfacet,
                                           uint samplingFlags, uint checkFlags)
{
    return primeFullOpenPbrReflectivePdfImpl<false>(wi, wo, microfacet, samplingFlags, checkFlags);
}

public float primeFullOpenPbrReflectivePdfImpl<let Continuous : bool>(
    float3 wi, float3 wo, PrimeFullOpenPbrMicrofacet microfacet, uint samplingFlags,
    uint checkFlags)
{
    return primeFullOpenPbrReflectiveEvalBaseImpl<Continuous>(wi, wo, microfacet, samplingFlags,
                                                              checkFlags)
        .pdf;
}
