// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/contract/albedo.slang";

public struct PrimeDenoiseAlbedos
{
    public float3 diffuse;
    public float3 specular;
};
