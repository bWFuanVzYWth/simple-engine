// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/model/material.slang";

import openpbr.contract.flags;
import openpbr.contract.material;
import openpbr.math.frame;

// OpenPBR's default metallic material initializer.
// These are compact boundary utilities, not a retained generic closure graph.

namespace openpbr {
namespace kernel {
public func material_from_metallic(base_color: float3, roughness: float, metalness: float,
                                   normal: float3)
    ->FullMaterial {
    var frame : Frame = frame_from_normal(normal);
    var material : FullMaterial;
    material.weight.base = 1.0;
    material.weight.diffuse_roughness = 0.0;
    material.weight.specular = 1.0;
    material.weight.metalness = metalness;
    material.weight.subsurface = 0.0;
    material.weight.transmission = 0.0;
    material.weight.coat = 0.0;
    material.weight.fuzz = 0.0;
    material.weight.thin_film = 0.0;
    material.weight.diffraction = 0.0;
    material.geometry.thin_walled = 0u;
    material.geometry.thickness = 0.005;
    material.geometry.frame = frame;
    material.specular.color = float3(1.0);
    material.specular.roughness = roughness;
    material.specular.roughness_anisotropy = 0.0;
    material.specular.ior = 1.5;
    material.emission.luminance = float3(0.0);
    material.base.color = base_color;
    material.subsurface.color = float3(0.8);
    material.subsurface.radius = float3(0.05, 0.025, 0.0125);
    material.subsurface.scatter_anisotropy = 0.0;
    material.transmission.color = float3(1.0);
    material.transmission.depth = 0.0;
    material.transmission.scatter = float3(0.0);
    material.transmission.scatter_anisotropy = 0.0;
    material.transmission.dispersion_scale = 0.0;
    material.transmission.dispersion_abbe_number = 20.0;
    material.coat.color = float3(1.0);
    material.coat.roughness = 0.0;
    material.coat.roughness_anisotropy = 0.0;
    material.coat.ior = 1.6;
    material.coat.darkening = 1.0;
    material.coat.roughening = 1.0;
    material.coat.frame = frame;
    material.fuzz.color = float3(1.0);
    material.fuzz.roughness = 0.5;
    material.thin_film.thickness = 0.5;
    material.thin_film.ior = 1.4;
    material.diffraction.color = float3(1.0);
    material.diffraction.thickness = 0.5;
    material.diffraction.inv_pitch = float2(1.0 / 3.0, 0.0);
    material.diffraction.angle = 0.0;
    material.diffraction.lobe_count = 5u;
    material.diffraction.kind = DIFFRACTION_RECTANGULAR;
    return material;
}
}
}
