// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/service/components.slang";

namespace openpbr {
namespace kernel {
public struct SurfaceComponents {
    public var diffuse_response : float3;
    public var specular_response : float3;
    public var pdf : float;
};
}
}
