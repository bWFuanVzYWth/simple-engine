// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/pt_subsurface.slang";

// Source RGB denotes the desired diffuse reflection. The thin SSS branch keeps
// that reflection and the old half-color transmission while absorption allows it.
// Scale all channels together when the derived albedo reaches one.
namespace openpbr
{
namespace kernel
{
public func subsurface_color(color: float3)->float3
{
    let maximum : float = max(color.x, max(color.y, color.z));
    let gain : float = 1.5 / max(1.0, 1.5 * maximum);
    return color * gain;
}

public static const let FULL_PT_SUBSURFACE_ANISOTROPY : float = -1.0 / 3.0;
}
}
