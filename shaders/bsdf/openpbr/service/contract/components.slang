// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/contract/components.slang";

public struct PrimeBsdfComponents
{
    public var diffuseResponse : float3;
    public var specularResponse : float3;
    public var pdf : float;
};
