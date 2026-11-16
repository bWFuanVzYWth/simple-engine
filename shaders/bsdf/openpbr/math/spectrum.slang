// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/spectrum.slang";
import math.constants;

namespace openpbr
{
namespace kernel
{
public static const let denominator_tolerance = 1.0e-10;

public func square(value: float)->float
{
    return value * value;
}
public func square(value: float2)->float2
{
    return value * value;
}
public func square(value: float3)->float3
{
    return value * value;
}
public func pow4(value: float)->float
{
    let x : float = value * value;
    return x * x;
}
public func pow5(value: float)->float
{
    return pow4(value) * value;
}
public func pow6(value: float)->float
{
    let x : float = value * value * value;
    return x * x;
}
public func reduce_sum(value: float2)->float
{
    return value.x + value.y;
}
public func reduce_sum(value: float3)->float
{
    return value.x + value.y + value.z;
}
public func reduce_max(value: float2)->float
{
    return max(value.x, value.y);
}
public func reduce_max(value: float3)->float
{
    return max(value.x, max(value.y, value.z));
}
public func reduce_min(value: float3)->float
{
    return min(value.x, min(value.y, value.z));
}
public func spectrum_to_weight(value: float3)->float
{
    return reduce_sum(value) * (1.0 / 3.0);
}

public func uniform_sample_hemisphere(sample_value: float2)->float3
{
    let sine : float = sqrt(1.0 - sample_value.x * sample_value.x);
    let phi : float = math::tau * sample_value.y;
    return float3(sine * cos(phi), sine * sin(phi), sample_value.x);
}

public func cosine_sample_hemisphere(sample_value: float2)->float3
{
    let radius : float = sqrt(sample_value.x);
    let phi : float = math::tau * sample_value.y;
    return float3(radius * cos(phi), radius * sin(phi), sqrt(1.0 - sample_value.x));
}

public func f0_to_ior(f0: float)->float
{
    let root : float = sqrt(clamp(f0, 0.0, 0.999));
    return (1.0 + root) / (1.0 - root);
}

public func f0_to_ior(f0: float3)->float3
{
    let root : float3 = sqrt(clamp(f0, float3(0.0), float3(0.999)));
    return (float3(1.0) + root) / (float3(1.0) - root);
}

public func ior_to_f0(ior: float)->float
{
    return square((ior - 1.0) / (ior + 1.0));
}

public func specular_ndf_roughnesses(roughness: float, anisotropy: float)->float2
{
    var alpha : float2;
    if (anisotropy == 0.0)
        return float2(square(roughness));
    alpha.x = square(roughness) * sqrt(2.0 / (1.0 + square(1.0 - anisotropy)));
    alpha.y = (1.0 - anisotropy) * alpha.x;
    if (reduce_max(alpha) < 1.0e-4)
        return float2(0.0);
    return max(alpha, float2(1.0e-4));
}

public func ior_adjustment(ior: float, coat_ior: float, coat_weight: float)->float
{
    if (coat_weight == 0.0)
        return ior;
    let boundary : float = coat_ior > ior ? coat_ior / ior : ior / coat_ior;
    return lerp(ior, boundary, coat_weight);
}

public func ior_adjustment(ior: float, specular_weight: float)->float
{
    let e : float = sign(ior - 1.0) * sqrt(clamp(specular_weight * ior_to_f0(ior), 0.0, 0.999));
    return (1.0 + e) / (1.0 - e);
}

public func thin_dielectric_roughness_scaler2(ior: float)->float
{
    let eta : float = ior >= 1.0 ? ior : 1.0 / ior;
    return 3.7 * (eta - 1.0) * square(eta - 0.5) / (eta * eta * eta);
}
}
}
