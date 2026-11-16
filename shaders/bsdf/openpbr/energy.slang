// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/energy.slang";

import bsdf.openpbr.math.microfacet_energy;

// Immutable author-supplied 44x32x159 HALF4 energy table, uploaded once into
// device-local image memory. Normalized linear clamp filtering matches legacy.

namespace openpbr
{
namespace kernel
{
public struct CombinedEnergySampler : openpbr::IEnergyLut
{
    public var texture : Sampler3D<float4>;
    public func lookup(uvw: float3)->float4
    {
        return texture.SampleLevel(uvw, 0.0);
    }
};
}
}
