// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/material.slang";

namespace openpbr
{
namespace kernel
{
public struct Frame
{
    public var tangent : float3;
    public var bitangent : float3;
    public var normal : float3;
};

public struct MaterialWeights
{
    public var base : float;
    public var diffuse_roughness : float;
    public var specular : float;
    public var metalness : float;
    public var subsurface : float;
    public var transmission : float;
    public var coat : float;
    public var fuzz : float;
    public var thin_film : float;
    public var diffraction : float;
};

public struct Geometry
{
    public var thin_walled : uint;
    public var thickness : float;
    public var frame : Frame;
};

public struct Specular
{
    public var color : float3;
    public var roughness : float;
    public var roughness_anisotropy : float;
    public var ior : float;
};

public struct Emission
{
    public var luminance : float3;
};
public struct Base
{
    public var color : float3;
};

public struct Subsurface
{
    public var color : float3;
    public var radius : float3;
    public var scatter_anisotropy : float;
};

public struct Transmission
{
    public var color : float3;
    public var depth : float;
    public var scatter : float3;
    public var scatter_anisotropy : float;
    public var dispersion_scale : float;
    public var dispersion_abbe_number : float;
};

public struct Coat
{
    public var color : float3;
    public var roughness : float;
    public var roughness_anisotropy : float;
    public var ior : float;
    public var darkening : float;
    public var roughening : float;
    public var frame : Frame;
};

public struct Fuzz
{
    public var color : float3;
    public var roughness : float;
};

public struct ThinFilm
{
    public var thickness : float;
    public var ior : float;
};

public struct Diffraction
{
    public var color : float3;
    public var thickness : float;
    public var inv_pitch : float2;
    public var angle : float;
    public var lobe_count : uint;
    public var kind : uint;
};

public struct FullMaterial
{
    public var weight : MaterialWeights;
    public var geometry : Geometry;
    public var specular : Specular;
    public var emission : Emission;
    public var base : Base;
    public var subsurface : Subsurface;
    public var transmission : Transmission;
    public var coat : Coat;
    public var fuzz : Fuzz;
    public var thin_film : ThinFilm;
    public var diffraction : Diffraction;
};
}
}
