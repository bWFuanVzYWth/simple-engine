/// 使用通用辅助量生产实现验证物理能量与重投影约定。
module denoise_test;

import pt.integrator;
import geometry.ray;
import openpbr.api;
import openpbr.denoise;
import openpbr.flow;
import openpbr.specialized;
import "openpbr/math/microfacet_energy.slang";
import camera.pinhole;
import denoise.guide;
import light_sampling.sample;
import pt.surface_guides;
import transport.surface_material;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var unused_energy : Texture3D<float4>;
[[vk::binding(3, 0)]]
var unused_sampler : SamplerState;

using namespace pt;

public struct ConstantEnergy : openpbr::IEnergyLut {
    public func lookup(uvw: float3)->float4 {
        return float4(0.2, 0.3, 0.4, 0.5);
    }
};

func shifted_camera(x: float)->camera::Camera {
    return { float4(x, 0, 0, 1), float4(0, 0, 1, 2), float4(1, 0, 0, 0), float4(0, 1, 0, 0) };
}

[numthreads(64, 1, 1)]
func denoise_motion_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    let extent = uint2(800, 400);
    let uv = float2(0.5) + float2(0.25, -0.125) / float2(extent);
    let direction : float3 = camera::camera_direction(shifted_camera(0), uv);
    let point : float3 = direction * (10.0 / direction.z);
    var motion : float2;
    var valid : bool = denoise::denoise_motion(shifted_camera(0), uv, point, false, extent, motion);
    outputs[0] = float4(motion, camera::camera_depth(shifted_camera(0), point), float(valid));
    valid = denoise::denoise_motion(shifted_camera(-1), uv, point, false, extent, motion);
    outputs[1] = float4(motion, 0, float(valid));
    valid = denoise::denoise_motion(shifted_camera(1), uv, point, false, extent, motion);
    outputs[2] = float4(motion, 0, float(valid));
    valid = denoise::denoise_motion(shifted_camera(-1), uv, direction, true, extent, motion);
    outputs[3] = float4(motion, denoise::DENOISE_SKY_DEPTH, float(valid));
    valid = denoise::denoise_motion(shifted_camera(0), uv, -point, false, extent, motion);
    outputs[4] = float4(motion, 0, float(valid));
}

[numthreads(64, 1, 1)]
func denoise_albedo_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    var material : transport::SurfaceMaterial = {};
    material.pbr.base_roughness = inputs[0];
    material.pbr.surface = float4(1, 0, 0, 0);
    material.transmitted = float4(1, 0, 0, 0);
    let matched : denoise::DenoiseGuides =
                      pt::denoise_surface(ConstantEnergy(), material, float3(0, 0, -1),
                                          float3(0, 0, 1), float4(1, 0, 0, 0));
    outputs[0] = matched.normal_roughness;
    outputs[1] = float4(matched.diffuse_albedo, 1);
    outputs[2] = float4(matched.specular_albedo, 1);
    material.pbr.surface.x = 1.5;
    let dielectric : denoise::DenoiseGuides =
                         pt::denoise_surface(ConstantEnergy(), material, float3(0, 0, 1),
                                             float3(0, 0, 1), float4(1, 0, 0, 0));
    outputs[3] = float4(dielectric.diffuse_albedo, 1);
    outputs[4] = float4(dielectric.specular_albedo, 1);
    material.pbr.control.x = 1;
    let solid : denoise::DenoiseGuides =
                    pt::denoise_surface(ConstantEnergy(), material, float3(0, 0, 1),
                                        float3(0, 0, 1), float4(1, 0, 0, 0));
    outputs[5] = float4(solid.diffuse_albedo, 1);
    outputs[6] = float4(solid.specular_albedo, 1);
}

func compare_flow_albedo<let flow : uint>(material: openpbr::Material, incident: float4,
                                          transmitted: float4, index: uint)
    ->void {
    let normal = float3(0, 0, 1);
    let view : float3 = normalize(float3(0.7, 0.1, 0.7));
    let reference : openpbr::DenoiseAlbedos = openpbr::denoise_albedos(
                        ConstantEnergy(), material, normal, view, incident, transmitted);
    let specialized : openpbr::DenoiseAlbedos = openpbr::denoise_albedos_flow<flow>(
                          ConstantEnergy(), material, normal, view, incident, transmitted);
    outputs[index * 4] = float4(reference.diffuse, 1);
    outputs[index * 4 + 1] = float4(reference.specular, 1);
    outputs[index * 4 + 2] = float4(specialized.diffuse, 1);
    outputs[index * 4 + 3] = float4(specialized.specular, 1);
}

[numthreads(64, 1, 1)]
func denoise_flow_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    var material : openpbr::Material = {};
    material.base_roughness = inputs[0];
    material.surface.x = 1;
    let air = float4(1, 0, 0, 0);
    let glass = float4(1.5, 0, 0, 0);
    compare_flow_albedo<1>(material, air, air, 0);
    material.surface.x = 1.5;
    compare_flow_albedo<131>(material, air, air, 1);
    material.surface.z = 0.5;
    compare_flow_albedo<139>(material, air, air, 2);
    material.surface.y = 1;
    material.surface.z = 0;
    compare_flow_albedo<132>(material, air, air, 3);
    material.surface.y = 0;
    material.control.x = 1;
    material.base_roughness.w = 0;
    compare_flow_albedo<338>(material, air, glass, 4);
    material.base_roughness.w = inputs[0].w;
    material.control.x = 3;
    compare_flow_albedo<674>(material, glass, air, 5);
    material.control.x = 0;
    material.surface.y = 0.5;
    compare_flow_albedo<2183>(material, air, air, 6);
}

[numthreads(64, 1, 1)]
func flow_discrete_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    var material : openpbr::Material = {};
    material.surface = float4(1, 0.5, 0, 0);
    let air = float4(1, 0, 0, 0);
    let glass = float4(1.5, 0, 0, 0);
    outputs[0] = float4(float(openpbr::is_discrete_only(material, air, air)),
                        float(openpbr::is_discrete_only_flow<2116>(material, air, air)), 0, 0);
    material.surface.y = 1;
    outputs[1] = float4(float(openpbr::is_discrete_only(material, air, air)),
                        float(openpbr::is_discrete_only_flow<68>(material, air, air)), 0, 0);
    material.control.x = 1;
    material.base_roughness.w = 0.5;
    outputs[2] = float4(float(openpbr::is_discrete_only(material, air, glass)),
                        float(openpbr::is_discrete_only_flow<658>(material, air, glass)), 0, 0);
    outputs[3] = float4(float(openpbr::is_discrete_only(material, glass, glass)),
                        float(openpbr::is_discrete_only_flow<272>(material, glass, glass)), 0, 0);
}

public struct PrefixScene : ITraceScene {
    public func delta_interface(hit: Hit, incident: float4)->DeltaInterface {
        return { hit.surface.normal, incident, float3(1) };
    }
    public func resolve(hit: Hit, capability: uint, incident: float4)->transport::SurfaceMaterial {
        var result : transport::SurfaceMaterial = {};
        result.pbr.base_roughness = float4(1, 1, 1, 0);
        result.pbr.surface.x = 1;
        result.pbr.control.x = 1;
        result.transmitted = incident;
        result.boundary_filter = 1;
        return result;
    }
    public func energy()->openpbr::Energy {
        return { unused_energy, unused_sampler };
    }
    public func sample_sun(point_value: geometry::SurfacePoint, sample_value: float2)
        ->light_sampling::LightSample {
        return (light_sampling::LightSample)0;
    }
    public func sample_light(point_value: geometry::SurfacePoint, sample_value: float3)
        ->light_sampling::LightSample {
        return (light_sampling::LightSample)0;
    }
    public func light_emission(point_value: geometry::SurfacePoint,
                               sample_value: light_sampling::LightSample, solar: bool)
        ->float3 {
        return sample_value.emission;
    }
    public func emission_pdf(hit: Hit, previous: float3)->float {
        return 0;
    }
    public func occluded(ray: geometry::Ray)->bool {
        return false;
    }
    public func environment(direction: float3, previous: float3, depth: uint)->Environment {
        return (Environment)0;
    }
    public func intersect(origin: float3, direction: float3, out hit: Hit)->bool {
        hit = {};
        hit.surface = { float3(0, 0, origin.z == 0 ? 2 : 4), float3(0, 0, -1), 0 };
        hit.distance = 2;
        hit.identity.x = origin.z == 0 ? 11 : 22;
        return true;
    }
    public func resolve_hit(inout hit: Hit, incident: float4)->void {
    }
    public func prefix(hit: Hit)->bool {
        return hit.identity.x == 11;
    }
    public func shade(inout data: TransportCall)->void {
    }
};

public struct PrefixGuides : IPrefixGuides {
    public func surface(data: TransportCall)->void {
        outputs[0] = float4(data.hit.surface.position, float(data.hit.identity.x));
    }
    public func environment(data: TransportCall)->void {
        outputs[0] = -1;
    }
};

[numthreads(64, 1, 1)]
func denoise_prefix_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    var with_guides : TransportCall = {};
    with_guides.path.direction = float3(0, 0, 1);
    with_guides.path.medium = float4(1, 0, 0, 0);
    with_guides.path.throughput = 1;
    with_guides.path.eta_scale = 1;
    var without_guides : TransportCall = with_guides;
    let settings : Settings = { 0, 3, 8, 8, false };
    var first : float;
    var other_first : float;
    trace_prefix(PrefixScene(), with_guides, settings, first, PrefixGuides());
    trace_prefix(PrefixScene(), without_guides, settings, other_first);
    outputs[1] = float4(with_guides.hit.surface.position, float(with_guides.hit.identity.x));
    outputs[2] = float4(with_guides.path.throughput, first);
    outputs[3] = float4(without_guides.path.throughput, other_first);
    outputs[4] = float4(with_guides.path.direction, float(with_guides.path.depth));
    outputs[5] = float4(without_guides.path.direction, float(without_guides.path.depth));
}
