/// 由路径的真实首命中生成表面导引。
module surface_guides;
import denoise.guide;
import openpbr.denoise;
import openpbr.contract.energy_lut;
import transport.surface_material;

namespace pt {
/// 从实际首命中翻译结果生成材质辅助量；只读，不改变积分器或随机消费。
public func denoise_surface<Energy>(energy: Energy, material: transport::SurfaceMaterial,
                                    normal: float3, view: float3, incident: float4)
    ->denoise::DenoiseGuides
    where Energy : openpbr::IEnergyLut {
    let albedos : openpbr::DenoiseAlbedos = openpbr::denoise_albedos(
                      energy, material.pbr, normal, view, incident, material.transmitted);
    var result : denoise::DenoiseGuides;
    result.normal_roughness =
        float4(dot(normal, view) < 0 ? -normal : normal, material.pbr.base_roughness.w);
    result.diffuse_albedo = albedos.diffuse;
    result.specular_albedo = albedos.specular;
    return result;
}

/// 相同辅助量的Flow专用入口；键由资源发布者证明，保留完整FP32方向能量。
public func denoise_surface_flow<let flow : uint, Energy>(energy: Energy,
                                                          material: transport::SurfaceMaterial,
                                                          normal: float3, view: float3,
                                                          incident: float4)
    ->denoise::DenoiseGuides
    where Energy : openpbr::IEnergyLut {
    let albedos : openpbr::DenoiseAlbedos = openpbr::denoise_albedos_flow<flow>(
                      energy, material.pbr, normal, view, incident, material.transmitted);
    return { float4(dot(normal, view) < 0 ? -normal : normal, material.pbr.base_roughness.w),
             albedos.diffuse, albedos.specular };
}
}
