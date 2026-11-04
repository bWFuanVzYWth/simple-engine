// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/frame.slang";

import "bsdf/openpbr/contract/material.slang";

public PrimeFullOpenPbrOnb primeFullOpenPbrOnbFromNormal(float3 normal)
{
    float signZ = normal.z >= 0.0 ? 1.0 : -1.0;
    float a = -1.0 / (signZ + normal.z);
    float b = normal.x * normal.y * a;
    PrimeFullOpenPbrOnb result;
    result.tangent = float3(1.0 + signZ * normal.x * normal.x * a, signZ * b, -signZ * normal.x);
    result.bitangent = float3(b, signZ + normal.y * normal.y * a, -normal.y);
    result.normal = normal;
    return result;
}

public float3 primeFullOpenPbrOnbToWorld(PrimeFullOpenPbrOnb onb, float3 value)
{
    return value.x * onb.tangent + value.y * onb.bitangent + value.z * onb.normal;
}

public float3 primeFullOpenPbrOnbToLocal(PrimeFullOpenPbrOnb onb, float3 value)
{
    return float3(dot(onb.tangent, value), dot(onb.bitangent, value), dot(onb.normal, value));
}
