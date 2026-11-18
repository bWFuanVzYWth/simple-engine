// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/math/frame.slang";

import openpbr.contract.material;

namespace openpbr {
namespace kernel {
public func frame_from_normal(normal: float3)->Frame {
    let sign_z : float = normal.z >= 0.0 ? 1.0 : -1.0;
    let a : float = -1.0 / (sign_z + normal.z);
    let b : float = normal.x * normal.y * a;
    var result : Frame;
    result.tangent = float3(1.0 + sign_z * normal.x * normal.x * a, sign_z * b, -sign_z * normal.x);
    result.bitangent = float3(b, sign_z + normal.y * normal.y * a, -normal.y);
    result.normal = normal;
    return result;
}

public func to_world(frame: Frame, value: float3)->float3 {
    return value.x * frame.tangent + value.y * frame.bitangent + value.z * frame.normal;
}

public func to_local(frame: Frame, value: float3)->float3 {
    return float3(dot(frame.tangent, value), dot(frame.bitangent, value), dot(frame.normal, value));
}
}
}
