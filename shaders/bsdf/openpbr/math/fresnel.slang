// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/fresnel.slang";

import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";

// Exact Fresnel subset used by the supported OpenPBR topologies.
// Thin film is omitted because the compact support predicate rejects it.

public func primeFullOpenPbrDielectricRsRp(ior: float, cosineTheta: float)
    ->PrimeFullOpenPbrFloatPair
{
    var cosineTheta = cosineTheta;
    // Equal optical media have no interface. The general subtraction loses precision
    // near grazing incidence and is 0/0 at exactly zero cosine.
    if (ior == 1.0)
    {
        let zero : PrimeFullOpenPbrFloatPair = { 0.0, 0.0 };
        return zero;
    }
    var eta : float = ior;
    if (cosineTheta < 0.0)
    {
        cosineTheta = -cosineTheta;
        eta = 1.0 / eta;
    }
    let eta2 : float = eta * eta;
    let t0 : float = cosineTheta * cosineTheta + (eta - 1.0) * (eta + 1.0);
    if (t0 <= 0)
        return { -1.0, 1.0 };
    let t1 : float = t0 > 0.0 ? sqrt(t0) : 0.0;
    // Multiply Rp's numerator and denominator by eta; IOR is strictly positive.
    let t2 : float = eta2 * cosineTheta;
    var result : PrimeFullOpenPbrFloatPair;
    result.first = (cosineTheta - t1) / (cosineTheta + t1);
    result.second = (t1 - t2) / (t1 + t2);
    return result;
}

public func primeFullOpenPbrDielectricReflectance(ior: float, cosineTheta: float)
    ->PrimeFullOpenPbrFloatPair
{
    var result : PrimeFullOpenPbrFloatPair = primeFullOpenPbrDielectricRsRp(ior, cosineTheta);
    result.first *= result.first;
    result.second *= result.second;
    return result;
}

public func primeFullOpenPbrDielectricUnpolarized(ior: float, cosineTheta: float)->float
{
    let value : PrimeFullOpenPbrFloatPair = primeFullOpenPbrDielectricReflectance(ior, cosineTheta);
    return 0.5 * (value.first + value.second);
}

public func primeFullOpenPbrDielectricRefract(ior: float, wi: float3, normal: float3)
    ->PrimeFullOpenPbrRefractResult
{
    var normal = normal;
    var cosineTheta : float = dot(normal, wi);
    var inverseEta : float;
    if (cosineTheta < 0.0)
    {
        inverseEta = 1.0 / ior;
    }
    else
    {
        cosineTheta = -cosineTheta;
        normal = -normal;
        inverseEta = ior;
    }
    let etaDifference : float = (1.0 - inverseEta) * (1.0 + inverseEta);
    let scaledCosine : float = inverseEta * cosineTheta;
    let t0 : float = scaledCosine * scaledCosine + etaDifference;
    var result : PrimeFullOpenPbrRefractResult;
    result.valid = t0 > 0.0 ? 1u : 0u;
    result.relativeIor = 1.0 / inverseEta;
    let root : float = sqrt(max(t0, 0.0));
    let normalChange : float = t0 > 0 ? etaDifference / (root - scaledCosine) : scaledCosine;
    result.wo = inverseEta == 1 ? wi : inverseEta * wi - normalChange * normal;
    return result;
}

/// 单位入射／出射方向及实际折射半向量的 Snell 恒等式；ior!=1。
/// 消去 cos_o+cos_i/eta 在近匹配 IOR 处的相消，保留折射 Jacobian 的真实尖峰。
public func primeFullOpenPbrRefractionDenominator(eta: float, cosineIncident: float,
                                                  cosineOutgoing: float)
    ->float
{
    return ((1 - eta) * (1 + eta)) / (eta * eta * (cosineIncident / eta - cosineOutgoing));
}

func primeFullOpenPbrSchlickF82Tint(f0: float3, f82: float3, f90: float3, weight: float,
                                    cosineTheta: float)
    ->float3
{
    let cosineThetaMax : float = 1.0 / 7.0;
    let cosineThetaFactor : float =
                                1.0 / (cosineThetaMax * primeFullOpenPbrPow6(1.0 - cosineThetaMax));
    let a : float3 = lerp(f0, f90, primeFullOpenPbrPow5(1.0 - cosineThetaMax)) *
                     (float3(1.0) - f82) * cosineThetaFactor;
    return clamp((lerp(f0, f90, primeFullOpenPbrPow5(1.0 - cosineTheta)) -
                  a * cosineTheta * primeFullOpenPbrPow6(1.0 - cosineTheta)) *
                     weight,
                 float3(0.0), float3(1.0));
}

public func primeFullOpenPbrSpecularFresnelUnpolarized(fresnel: PrimeFullOpenPbrSpecularFresnel,
                                                       cosineTheta: float)
    ->float3
{
    return float3(primeFullOpenPbrDielectricUnpolarized(fresnel.energyIor, cosineTheta)) *
           fresnel.color;
}

public func primeFullOpenPbrSpecularFresnelRt(fresnel: PrimeFullOpenPbrSpecularFresnel,
                                              cosineTheta: float)
    ->PrimeFullOpenPbrVecPair
{
    var cosineTheta = cosineTheta;
    var tint = true;
    if (cosineTheta < 0.0)
    {
        let t0 : float = 1.0 - (1.0 - cosineTheta * cosineTheta) * fresnel.ior * fresnel.ior;
        if (t0 > 0.0)
        {
            cosineTheta = sqrt(t0);
        }
        else
        {
            var totalInternalReflection : PrimeFullOpenPbrVecPair;
            totalInternalReflection.first = float3(1.0);
            totalInternalReflection.second = float3(0.0);
            return totalInternalReflection;
        }
        tint = false;
    }
    let reflection = float3(primeFullOpenPbrDielectricUnpolarized(fresnel.energyIor, cosineTheta));
    var result : PrimeFullOpenPbrVecPair;
    result.second = float3(1.0) - reflection;
    result.first = tint ? reflection * fresnel.color : reflection;
    return result;
}

public func primeFullOpenPbrSpecularFresnelRefract(fresnel: PrimeFullOpenPbrSpecularFresnel,
                                                   wi: float3, normal: float3)
    ->PrimeFullOpenPbrRefractResult
{
    return primeFullOpenPbrDielectricRefract(fresnel.ior, wi, normal);
}

public func primeFullOpenPbrConductorFresnelUnpolarized(fresnel: PrimeFullOpenPbrConductorFresnel,
                                                        cosineTheta: float)
    ->float3
{
    return primeFullOpenPbrSchlickF82Tint(fresnel.f0, fresnel.f82, fresnel.f90, fresnel.weight,
                                          cosineTheta);
}
