// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/spectrum.slang";

public static const let PRIME_FULL_OPENPBR_PI = 3.14159265358979323846;
public static const let PRIME_FULL_OPENPBR_INV_PI = 0.31830988618379067154;
public static const let PRIME_FULL_OPENPBR_DENOM_TOLERANCE = 1.0e-10;

public func primeFullOpenPbrSquare(value: float)->float
{
    return value * value;
}
public func primeFullOpenPbrSquare(value: float2)->float2
{
    return value * value;
}
public func primeFullOpenPbrSquare(value: float3)->float3
{
    return value * value;
}
public func primeFullOpenPbrPow4(value: float)->float
{
    let x : float = value * value;
    return x * x;
}
public func primeFullOpenPbrPow5(value: float)->float
{
    return primeFullOpenPbrPow4(value) * value;
}
public func primeFullOpenPbrPow6(value: float)->float
{
    let x : float = value * value * value;
    return x * x;
}
public func primeFullOpenPbrReduceSum(value: float2)->float
{
    return value.x + value.y;
}
public func primeFullOpenPbrReduceSum(value: float3)->float
{
    return value.x + value.y + value.z;
}
public func primeFullOpenPbrReduceMax(value: float2)->float
{
    return max(value.x, value.y);
}
public func primeFullOpenPbrReduceMax(value: float3)->float
{
    return max(value.x, max(value.y, value.z));
}
public func primeFullOpenPbrReduceMin(value: float3)->float
{
    return min(value.x, min(value.y, value.z));
}
public func primeFullOpenPbrSpectrumToWeight(value: float3)->float
{
    return primeFullOpenPbrReduceSum(value) / 3.0;
}

public func primeFullOpenPbrUniformSampleHemisphere(sampleValue: float2)->float3
{
    let sine : float = sqrt(1.0 - sampleValue.x * sampleValue.x);
    let phi : float = 2.0 * PRIME_FULL_OPENPBR_PI * sampleValue.y;
    return float3(sine * cos(phi), sine * sin(phi), sampleValue.x);
}

public func primeFullOpenPbrCosineSampleHemisphere(sampleValue: float2)->float3
{
    let radius : float = sqrt(sampleValue.x);
    let phi : float = 2.0 * PRIME_FULL_OPENPBR_PI * sampleValue.y;
    return float3(radius * cos(phi), radius * sin(phi), sqrt(1.0 - sampleValue.x));
}

public func primeFullOpenPbrF0ToIor(f0: float)->float
{
    let root : float = sqrt(clamp(f0, 0.0, 0.999));
    return (1.0 + root) / (1.0 - root);
}

public func primeFullOpenPbrF0ToIor(f0: float3)->float3
{
    let root : float3 = sqrt(clamp(f0, float3(0.0), float3(0.999)));
    return (float3(1.0) + root) / (float3(1.0) - root);
}

public func primeFullOpenPbrIorToF0(ior: float)->float
{
    return primeFullOpenPbrSquare((ior - 1.0) / (ior + 1.0));
}

public func primeFullOpenPbrSpecularNdfRoughnesses(roughness: float, anisotropy: float)->float2
{
    var alpha : float2;
    if (anisotropy == 0.0)
        return float2(primeFullOpenPbrSquare(roughness));
    alpha.x = primeFullOpenPbrSquare(roughness) *
              sqrt(2.0 / (1.0 + primeFullOpenPbrSquare(1.0 - anisotropy)));
    alpha.y = (1.0 - anisotropy) * alpha.x;
    if (primeFullOpenPbrReduceMax(alpha) < 1.0e-4)
        return float2(0.0);
    return max(alpha, float2(1.0e-4));
}

public func primeFullOpenPbrIorAdjustment(ior: float, coatIor: float, coatWeight: float)->float
{
    if (coatWeight == 0.0)
        return ior;
    let boundary : float = coatIor > ior ? coatIor / ior : ior / coatIor;
    return lerp(ior, boundary, coatWeight);
}

public func primeFullOpenPbrIorAdjustment(ior: float, specularWeight: float)->float
{
    let e : float = sign(ior - 1.0) *
                    sqrt(clamp(specularWeight * primeFullOpenPbrIorToF0(ior), 0.0, 0.999));
    return (1.0 + e) / (1.0 - e);
}

public func primeFullOpenPbrThinDielectricRoughnessScaler2(ior: float)->float
{
    let eta : float = ior >= 1.0 ? ior : 1.0 / ior;
    return 3.7 * (eta - 1.0) * primeFullOpenPbrSquare(eta - 0.5) / (eta * eta * eta);
}
