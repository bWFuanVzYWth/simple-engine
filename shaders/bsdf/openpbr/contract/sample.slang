// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/sample.slang";

import "bsdf/openpbr/contract/flags.slang";

public struct PrimeFullOpenPbrThroughput
{
    public var value : float3;
    public var flags : uint;
};

public struct PrimeFullOpenPbrSample
{
    public var throughput : PrimeFullOpenPbrThroughput;
    public var pdf : float;
    public var eta : float;
    public var wo : float3;
};

public struct PrimeFullOpenPbrFloatPair
{
    public var first : float;
    public var second : float;
};
public struct PrimeFullOpenPbrVecPair
{
    public var first : float3;
    public var second : float3;
};

public struct PrimeFullOpenPbrRefractResult
{
    public var wo : float3;
    public var relativeIor : float;
    public var valid : uint;
};

public struct PrimeFullOpenPbrMicrofacet
{
    public var alpha : float2;
};

public struct PrimeFullOpenPbrSpecularFresnel
{
    public var ior : float;
    public var energyIor : float;
    public var color : float3;
};

public struct PrimeFullOpenPbrConductorFresnel
{
    public var f0 : float3;
    // Relative Schlick tint at RoboCute's 1/7 cosine anchor, not absolute F(82 degrees).
    public var f82 : float3;
    public var f90 : float3;
    public var weight : float;
    public var energyF0 : float3;
};

public func primeFullOpenPbrZeroThroughput()->PrimeFullOpenPbrThroughput
{
    var result : PrimeFullOpenPbrThroughput;
    result.value = float3(0.0);
    result.flags = PRIME_FULL_OPENPBR_FLAG_NONE;
    return result;
}

public func primeFullOpenPbrZeroSample()->PrimeFullOpenPbrSample
{
    var result : PrimeFullOpenPbrSample;
    result.throughput = primeFullOpenPbrZeroThroughput();
    result.pdf = 0.0;
    result.eta = 1.0;
    result.wo = float3(0.0);
    return result;
}

public func primeFullOpenPbrThroughputAdd(a: PrimeFullOpenPbrThroughput,
                                          b: PrimeFullOpenPbrThroughput)
    ->PrimeFullOpenPbrThroughput
{
    var result : PrimeFullOpenPbrThroughput;
    result.value = a.value + b.value;
    result.flags = a.flags | b.flags;
    return result;
}

public func primeFullOpenPbrThroughputScale(value: PrimeFullOpenPbrThroughput, scale: float3)
    ->PrimeFullOpenPbrThroughput
{
    var value = value;
    value.value *= scale;
    if (all(scale == float3(0.0)))
        value.flags = PRIME_FULL_OPENPBR_FLAG_NONE;
    return value;
}

public func primeFullOpenPbrThroughputScale(value: PrimeFullOpenPbrThroughput, scale: float)
    ->PrimeFullOpenPbrThroughput
{
    var value = value;
    value.value *= scale;
    if (scale == 0.0)
        value.flags = PRIME_FULL_OPENPBR_FLAG_NONE;
    return value;
}
