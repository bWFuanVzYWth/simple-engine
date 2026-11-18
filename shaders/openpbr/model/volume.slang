// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/model/volume.slang";

import openpbr.contract.material;
import openpbr.contract.volume;
import openpbr.math.spectrum;

namespace openpbr {
namespace kernel {
public func volume_from_transmission(transmission: Transmission)->Volume {
    var result : Volume;
    if (transmission.depth > 0.0) {
        let mu_t : float3 = -log(max(float3(1.0e-3), transmission.color)) / transmission.depth;
        let mu_s : float3 = transmission.scatter / transmission.depth;
        var mu_a : float3 = mu_t - mu_s;
        mu_a -= float3(min(0.0, reduce_min(mu_a)));
        result.extinction = mu_a + mu_s;
        result.albedo = float3(result.extinction.x != 0.0 ? mu_s.x / result.extinction.x : 0.0,
                               result.extinction.y != 0.0 ? mu_s.y / result.extinction.y : 0.0,
                               result.extinction.z != 0.0 ? mu_s.z / result.extinction.z : 0.0);
        result.anisotropy = clamp(transmission.scatter_anisotropy, -0.99, 0.99);
    } else {
        result.extinction = float3(0.0);
        result.albedo = float3(0.0);
        result.anisotropy = 0.0;
    }
    result.ior = 1.0;
    result.medium_id = 0u;
    result.ior_encoding = 0u;
    return result;
}
}
}
