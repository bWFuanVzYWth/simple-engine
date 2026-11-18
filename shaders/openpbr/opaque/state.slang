// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/opaque/state.slang";

import openpbr.contract.material;
import openpbr.contract.sample;

namespace openpbr {
namespace kernel {
public static const let FULL_COMPACT_OPAQUE_DIELECTRIC = 0u;
public static const let FULL_COMPACT_OPAQUE_CONDUCTOR = 1u;
public static const let FULL_COMPACT_OPAQUE_SUBSURFACE = 2u;
public static const let FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE = 3u;

public struct MixState {
    public var first_sample_weight : float;
    public var second_sample_weight : float;
};

public struct LayerState {
    public var transmittance_in : float3;
    public var base_sample_weight : float;
    public var specular_sample_weight : float;
};

public struct OpaqueState {
    public var frame : Frame;
    public var kind : uint;
    public var metalness : float;
    public var base_color : float3;
    public var subsurface_weight : float;
    public var subsurface_color : float3;
    public var subsurface_anisotropy : float;
    public var microfacet : Microfacet;
    public var dielectric_fresnel : SpecularFresnel;
    public var dielectric_multiple_scattering : float3;
    public var conductor_fresnel : ConductorFresnel;
    public var conductor_ess : float;
    public var diffuse_mix : MixState;
    public var layer : LayerState;
};

public struct LocalEvaluation {
    public var response : Response;
    public var pdf : float;
};

public struct OpaqueComponents {
    public var diffuse : float3;
    public var specular : float3;
    public var pdf : float;
};

public struct OpaqueEnergy {
    public var diffuse : float3;
    public var specular : float3;
};
}
}
