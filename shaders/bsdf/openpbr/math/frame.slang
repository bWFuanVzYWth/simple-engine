// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/frame.slang";

import "bsdf/openpbr/contract/material.slang";

public func primeFullOpenPbrOnbFromNormal(normal: float3)->PrimeFullOpenPbrOnb
{
    let signZ : float = normal.z >= 0.0 ? 1.0 : -1.0;
    let a : float = -1.0 / (signZ + normal.z);
    let b : float = normal.x * normal.y * a;
    var result : PrimeFullOpenPbrOnb;
    result.tangent = float3(1.0 + signZ * normal.x * normal.x * a, signZ * b, -signZ * normal.x);
    result.bitangent = float3(b, signZ + normal.y * normal.y * a, -normal.y);
    result.normal = normal;
    return result;
}

public func primeFullOpenPbrOnbToWorld(onb: PrimeFullOpenPbrOnb, value: float3)->float3
{
    return value.x * onb.tangent + value.y * onb.bitangent + value.z * onb.normal;
}

public func primeFullOpenPbrOnbToLocal(onb: PrimeFullOpenPbrOnb, value: float3)->float3
{
    return float3(dot(onb.tangent, value), dot(onb.bitangent, value), dot(onb.normal, value));
}
