// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/math/microfacet_energy.slang";

import openpbr.contract.material;
import openpbr.contract.sample;
import openpbr.math.spectrum;

import math.constants;
__exported import openpbr.contract.energy_lut;

namespace openpbr {
namespace kernel {
func microfacet_alpha2(microfacet: Microfacet)->float {
    return 0.5 * reduce_sum(square(microfacet.alpha));
}

public func microfacet_directional_albedo_reflection<let isotropic : bool = false>(
    microfacet: Microfacet, cosine_theta: float, f0: float3, f90: float3)
    ->float3 {
    let ab = microfacet_directional_albedo_coefficients<isotropic>(microfacet, cosine_theta);
    return f0 * ab.x + f90 * ab.y;
}

public func microfacet_directional_albedo_coefficients<let isotropic : bool = false>(
    microfacet: Microfacet, cosine_theta: float)
    ->float2 {
    let x : float = cosine_theta;
    let y2 : float = microfacet_alpha2(microfacet);
    let y : float = isotropic ? microfacet.alpha.x : sqrt(y2);
    // The same biquadratic, with three independent x chains and no xy temporaries.
    let r0
        : float4 =
              (float4(29.34, 1.424, 28.96, 13.08) * x + float4(-0.6303, -2.323, -1.765, 0.2281)) *
                  x +
              float4(0.1003, 0.9345, 1.0, 1.0);
    let r1 : float4 =
                 (float4(-26.44, 1.436, -36.11, 54.9) * x + float4(-2.038, -3.748, 11.53, -55.83)) *
                     x +
                 float4(9.748, 2.229, 8.263, 15.94);
    let r2 : float4 =
                 (float4(-5.448, 0.6286, 33.37, -285.1) * x + float4(19.99, 0.2913, 15.86, 300.2)) *
                     x +
                 float4(-8.245, -0.7684, -7.507, 41.26);
    let r : float4 = (r2 * y + r1) * y + r0;
    let ab : float2 = clamp(r.xy / r.zw, float2(0.0), float2(1.0));
    return ab;
}

/// smooth=true 只供已证明 roughness²<1e-4 的 opaque delta
/// specular；微面固定为0，只有方向反射能量被消费。 z 的 LUT 坐标在该域固定为首层，仍保留原 base
/// 透过率与 proposal 权重。
/// 返回float3：x=反射补偿，y=透射补偿，z=方向反射能量；投影不改变坐标或查询次数。
public func dielectric_energy<let smooth : bool = false, let decode_reflection : bool = true,
                              let decode_transmission : bool = true, let isotropic : bool = false,
                              EnergyLut>(energy_lut: EnergyLut, microfacet: Microfacet,
                                         cosine_theta: float, relative_ior: float,
                                         perceptual_roughness: float = 0)
    ->float3
    where EnergyLut : openpbr::IEnergyLut {
    if (relative_ior == 1.0) {
        return float3(1.0, 1.0, 0.0);
    }

    let q_min = 5.960464477539063e-8;
    let q_critical = 0.00010002000500139488;
    let q_max = 0.999499874937461;
    let near_log2_scale = 0.09334822745469708;
    let far_log2_min : float = -13.287279491668832;
    let far_log2_scale = 0.04123374454764629;

    let inside : bool = (cosine_theta < 0.0) != (relative_ior < 1.0);
    let mu : float = clamp(abs(cosine_theta), 0.02, 1.0);
    let q : float = clamp(abs((relative_ior - 1.0) / (relative_ior + 1.0)), q_min, q_max);
    let critical : float = min(2.0 * sqrt(q) / (1.0 + q), 0.9999999403953552);
    let near_inside : bool = inside && q <= q_critical;
    let below_critical : bool = inside && !near_inside && mu < critical;

    let near_u : float = clamp((log2(q) + 24.0) * near_log2_scale, 0.0, 1.0);
    let near_z : float = near_u - 0.65 * near_u * (1.0 - near_u) * (2.0 * near_u - 1.0);
    let far_u : float = clamp((log2(q / (1.0 - q)) - far_log2_min) * far_log2_scale, 0.0, 1.0);
    let far_z : float = sqrt(far_u);
    let exterior_z : float = q <= q_critical ? (12.0 / 47.0) * near_z
                                             : (12.0 + 35.0 * far_z) * (1.0 / 47.0);
    let logical_z : float = inside ? (near_inside ? near_z : far_z) : exterior_z;
    let z_offset : float = inside ? (near_inside ? 48.0 : (below_critical ? 111.0 : 63.0)) : 0.0;
    let z_extent : float = near_inside ? 15.0 : 47.0;

    let roughness : float = smooth      ? 0.0
                            : isotropic ? perceptual_roughness
                                        : sqrt(sqrt(microfacet_alpha2(microfacet)));
    let roughness_coordinate
        : float = smooth ? 0.0 : sqrt(clamp((roughness - 0.01) * (1.0 / 0.99), 0.0, 1.0));
    let linear_view : float = (mu - 0.02) * (1.0 / 0.98);
    let below_view : float = (mu - 0.02) / max(critical - 0.02, 1.0e-20);
    let above_view : float = (mu - critical) / max(1.0 - critical, 1.0e-20);
    var view : float = clamp(below_critical ? below_view
                                            : (inside && !near_inside ? above_view : linear_view),
                             0.0, 1.0);
    view = view <= 0.5 ? 0.5 * sqrt(2.0 * view) : 1.0 - 0.5 * sqrt(2.0 * (1.0 - view));
    view -= 0.64 * view * (1.0 - view) * (2.0 * view - 1.0);

    let uvw = float3((view * 43.0 + 0.5) * (1.0 / 44.0),
                     (roughness_coordinate * 31.0 + 0.5) * (1.0 / 32.0),
                     (z_offset + logical_z * z_extent + 0.5) * (1.0 / 159.0));
    let encoded : float4 = energy_lut.lookup(uvw);

    var compensation_scale = float2(1);
    if (decode_reflection)
        compensation_scale.x = exp2(clamp(encoded.x, -80.0, 80.0));
    if (decode_transmission) {
        let a : float =
                    0.5 *
                    (sqrt(1.0 + microfacet_alpha2(microfacet) * (1.0 - mu * mu) / (mu * mu)) + 1.0);
        let inverse_a : float = 1.0 / a;
        let eta_one_log2_t
            : float = 1.0 - 2.0 * a + 0.5 * log2(math::pi * a) - 1.45676773e-5 +
                      inverse_a * (1.80676418e-1 +
                                   inverse_a * (-1.85532904e-3 - 4.61868116e-3 * inverse_a));
        let eta_q0 : float = 3.0e-4 + 6.82190321e-2 * roughness * mu;
        let eta_factor : float = eta_q0 / (q + eta_q0) * eta_one_log2_t;
        let q_fraction : float = frac(logical_z * z_extent);
        let transmission_correction : float =
                                          (2.0 / 3.0) * encoded.w * q_fraction * (1.0 - q_fraction);
        compensation_scale.y =
            exp2(clamp(encoded.y - eta_factor + transmission_correction, -80.0, 80.0));
    }
    let encoded_reflectance : float = clamp(encoded.z, 0.0, 1.0);
    let reflectance_endpoint : float = 1.0 - abs(2.0 * encoded_reflectance - 1.0);
    let reflectance_edge : float = 0.5 * square(reflectance_endpoint) * sqrt(reflectance_endpoint);
    let directional_reflectance : float = encoded_reflectance <= 0.5 ? reflectance_edge
                                                                     : 1.0 - reflectance_edge;
    return float3(compensation_scale, directional_reflectance);
}

public func microfacet_directional_albedo_unity<let isotropic : bool = false>(
    microfacet: Microfacet, cosine_theta: float)
    ->float {
    return microfacet_directional_albedo_reflection<isotropic>(microfacet, cosine_theta,
                                                               float3(1.0), float3(1.0))
        .x;
}
}
}
