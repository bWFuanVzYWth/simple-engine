// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/math/fresnel.slang";

import openpbr.contract.sample;
import openpbr.math.spectrum;

// Exact Fresnel subset used by the supported OpenPBR topologies.
// Thin film is omitted because the compact support predicate rejects it.

namespace openpbr {
namespace kernel {
public func dielectric_polarized_amplitude(ior: float, cosine_theta: float)->PolarizedValue {
    var cosine_theta = cosine_theta;
    // Equal optical media have no interface. The general subtraction loses precision
    // near grazing incidence and is 0/0 at exactly zero cosine.
    if (ior == 1.0) {
        let zero : PolarizedValue = { 0.0, 0.0 };
        return zero;
    }
    var eta : float = ior;
    if (cosine_theta < 0.0) {
        cosine_theta = -cosine_theta;
        eta = 1.0 / eta;
    }
    let eta2 : float = eta * eta;
    let t0 : float = cosine_theta * cosine_theta + (eta - 1.0) * (eta + 1.0);
    if (t0 <= 0)
        return { -1.0, 1.0 };
    let t1 : float = t0 > 0.0 ? sqrt(t0) : 0.0;
    // Multiply Rp's numerator and denominator by eta; IOR is strictly positive.
    let t2 : float = eta2 * cosine_theta;
    var result : PolarizedValue;
    result.s = (cosine_theta - t1) / (cosine_theta + t1);
    result.p = (t1 - t2) / (t1 + t2);
    return result;
}

public func dielectric_polarized_reflectance(ior: float, cosine_theta: float)->PolarizedValue {
    var result : PolarizedValue = dielectric_polarized_amplitude(ior, cosine_theta);
    result.s *= result.s;
    result.p *= result.p;
    return result;
}

public func dielectric_reflectance(ior: float, cosine_theta: float)->float {
    let value : PolarizedValue = dielectric_polarized_reflectance(ior, cosine_theta);
    return 0.5 * (value.s + value.p);
}

public func dielectric_refract(ior: float, wi: float3, normal: float3)->RefractResult {
    var normal = normal;
    var cosine_theta : float = dot(normal, wi);
    var inverse_eta : float;
    if (cosine_theta < 0.0) {
        inverse_eta = 1.0 / ior;
    } else {
        cosine_theta = -cosine_theta;
        normal = -normal;
        inverse_eta = ior;
    }
    let eta_difference : float = (1.0 - inverse_eta) * (1.0 + inverse_eta);
    let scaled_cosine : float = inverse_eta * cosine_theta;
    let t0 : float = scaled_cosine * scaled_cosine + eta_difference;
    var result : RefractResult;
    result.valid = t0 > 0.0 ? 1u : 0u;
    result.relative_ior = 1.0 / inverse_eta;
    let root : float = sqrt(max(t0, 0.0));
    let normal_change : float = t0 > 0 ? eta_difference / (root - scaled_cosine) : scaled_cosine;
    result.wo = inverse_eta == 1 ? wi : inverse_eta * wi - normal_change * normal;
    return result;
}

/// 单位入射／出射方向及实际折射半向量的 Snell 恒等式；ior!=1。
/// 消去 cos_o+cos_i/eta 在近匹配 IOR 处的相消，保留折射 Jacobian 的真实尖峰。
public func refraction_denominator(eta: float, cosine_incident: float, cosine_outgoing: float)
    ->float {
    return ((1 - eta) * (1 + eta)) / (eta * eta * (cosine_incident / eta - cosine_outgoing));
}

func schlick_f82_tint(f0: float3, f82: float3, f90: float3, weight: float, cosine_theta: float)
    ->float3 {
    let cosine_theta_max : float = 1.0 / 7.0;
    let cosine_theta_factor : float = 1.0 / (cosine_theta_max * pow6(1.0 - cosine_theta_max));
    let a : float3 = lerp(f0, f90, pow5(1.0 - cosine_theta_max)) * (float3(1.0) - f82) *
                     cosine_theta_factor;
    return clamp(
        (lerp(f0, f90, pow5(1.0 - cosine_theta)) - a * cosine_theta * pow6(1.0 - cosine_theta)) *
            weight,
        float3(0.0), float3(1.0));
}

public func specular_reflectance(fresnel: SpecularFresnel, cosine_theta: float)->float3 {
    return float3(dielectric_reflectance(fresnel.energy_ior, cosine_theta)) * fresnel.color;
}

public func specular_interface_response(fresnel: SpecularFresnel, cosine_theta: float)
    ->InterfaceResponse {
    var cosine_theta = cosine_theta;
    var tint = true;
    if (cosine_theta < 0.0) {
        let t0 : float = 1.0 - (1.0 - cosine_theta * cosine_theta) * fresnel.ior * fresnel.ior;
        if (t0 > 0.0) {
            cosine_theta = sqrt(t0);
        } else {
            var total_internal_reflection : InterfaceResponse;
            total_internal_reflection.reflection = float3(1.0);
            total_internal_reflection.transmission = float3(0.0);
            return total_internal_reflection;
        }
        tint = false;
    }
    let reflection = float3(dielectric_reflectance(fresnel.energy_ior, cosine_theta));
    var result : InterfaceResponse;
    result.transmission = float3(1.0) - reflection;
    result.reflection = tint ? reflection * fresnel.color : reflection;
    return result;
}

public func specular_fresnel_refract(fresnel: SpecularFresnel, wi: float3, normal: float3)
    ->RefractResult {
    return dielectric_refract(fresnel.ior, wi, normal);
}

public func conductor_reflectance(fresnel: ConductorFresnel, cosine_theta: float)->float3 {
    return schlick_f82_tint(fresnel.f0, fresnel.f82, fresnel.f90, fresnel.weight, cosine_theta);
}
}
}
