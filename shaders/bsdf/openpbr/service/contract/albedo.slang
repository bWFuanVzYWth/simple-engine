// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/contract/albedo.slang";

namespace openpbr
{
namespace kernel
{
public struct DenoiseAlbedos
{
    public var diffuse : float3;
    public var specular : float3;
};
}
}
