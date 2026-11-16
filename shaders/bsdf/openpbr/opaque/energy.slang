// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/energy.slang";

import bsdf.openpbr.opaque.state;
import bsdf.openpbr.opaque.lobes;
import bsdf.openpbr.contract.flags;

namespace openpbr
{
namespace kernel
{
public func opaque_energy(wi: float3, state: OpaqueState)->OpaqueEnergy
{
    var result : OpaqueEnergy;
    result.diffuse = float3(0.0);
    result.specular = float3(0.0);
    if (state.kind == FULL_COMPACT_OPAQUE_CONDUCTOR)
    {
        result.specular = conductor_energy(wi, state, LOBE_ALL);
        return result;
    }
    let base : float3 = state.kind == FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                            ? mixed_diffuse_energy(state, LOBE_DIFFUSE)
                        : state.kind == FULL_COMPACT_OPAQUE_SUBSURFACE
                            ? subsurface_energy(state, LOBE_DIFFUSE)
                            : lambert_energy(wi, state, LOBE_DIFFUSE);
    result.diffuse = base * state.layer.transmittance_in;
    result.specular = opaque_dielectric_energy(wi, state, LOBE_SPECULAR | LOBE_DELTA);
    if (state.metalness > 0.0)
    {
        result.diffuse *= 1.0 - state.metalness;
        result.specular =
            lerp(result.specular, conductor_energy(wi, state, LOBE_ALL), state.metalness);
    }
    return result;
}
}
}
