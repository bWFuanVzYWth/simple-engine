// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/contract.slang";

import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.volume;
import bsdf.openpbr.contract.sample;

namespace openpbr
{
namespace kernel
{
public static const let FULL_COMPACT_TRANSMISSION_WAVELENGTHS_NM = float3(630.0, 532.0, 467.0);

public struct TransmissionState
{
    public var frame : Frame;
    public var geometry_thin_walled : uint;
    public var entering : uint;
    public var ray_t : float;
    public var original_ior : float;
    public var transmission_color : float3;
    public var specular_microfacet : Microfacet;
    public var specular_fresnel : SpecularFresnel;
    public var transmission_microfacet : Microfacet;
    public var transmission_tint : float3;
    public var transmission_volume : Volume;
    public var transmission_multiple_scattering : float3;
};

public struct TransmissionEvaluation
{
    public var response : Response;
    public var pdf : float;
};

public func transmission_material_frame(w: float3, state: TransmissionState)->float3
{
    return state.entering != 0u ? w : float3(-w.x, w.y, -w.z);
}
}
}
