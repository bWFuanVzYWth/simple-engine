// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/contract/volume.slang";

import openpbr.contract.flags;

namespace openpbr {
namespace kernel {
public struct Volume {
    public var extinction : float3;
    public var albedo : float3;
    public var anisotropy : float;
    public var ior : float;
    public var medium_id : uint;
    public var ior_encoding : uint;
};

// A ray carries exactly one resolved medium. Zero is the explicit vacuum endpoint.
public func vacuum_volume()->Volume {
    var medium : Volume = {};
    medium.ior = 1.0;
    return medium;
}
}
}
