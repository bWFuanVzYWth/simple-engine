/// 使用通用辅助量生产实现验证物理能量与重投影约定。
module denoise_test;

import pt.denoise;
import pt.integrator;
import geometry.ray;
import bsdf.openpbr.api;
import bsdf.openpbr.specialized;
import "bsdf/openpbr/math/microfacet_energy.slang";

[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;
[[vk::binding(2, 0)]]
Texture3D<float4> unused_energy;
[[vk::binding(3, 0)]]
SamplerState unused_sampler;

using namespace pt;

public struct ConstantEnergy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    public float4 sampleLevel(float3 uvw)
    {
        return float4(0.2, 0.3, 0.4, 0.5);
    }
};

DenoiseCamera camera(float x)
{
    return { float4(x, 0, 0, 1), float4(0, 0, 1, 2), float4(1, 0, 0, 0), float4(0, 1, 0, 0) };
}

[numthreads(64, 1, 1)]
void denoise_motion_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    uint2 extent = uint2(800, 400);
    float2 uv = float2(0.5) + float2(0.25, -0.125) / float2(extent);
    float3 direction = denoise_direction(camera(0), uv);
    float3 point = direction * (10.0 / direction.z);
    float2 motion;
    bool valid = denoise_motion(camera(0), uv, point, false, extent, motion);
    outputs[0] = float4(motion, denoise_depth(camera(0), point), float(valid));
    valid = denoise_motion(camera(-1), uv, point, false, extent, motion);
    outputs[1] = float4(motion, 0, float(valid));
    valid = denoise_motion(camera(1), uv, point, false, extent, motion);
    outputs[2] = float4(motion, 0, float(valid));
    valid = denoise_motion(camera(-1), uv, direction, true, extent, motion);
    outputs[3] = float4(motion, DENOISE_SKY_DEPTH, float(valid));
    valid = denoise_motion(camera(0), uv, -point, false, extent, motion);
    outputs[4] = float4(motion, 0, float(valid));
}

[numthreads(64, 1, 1)]
void denoise_albedo_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    SurfaceMaterial material = {};
    material.pbr.base_roughness = inputs[0];
    material.pbr.surface = float4(1, 0, 0, 0);
    material.transmitted = float4(1, 0, 0, 0);
    DenoiseGuides matched = denoise_surface(ConstantEnergy(), material, float3(0, 0, -1),
                                            float3(0, 0, 1), float4(1, 0, 0, 0));
    outputs[0] = matched.normal_roughness;
    outputs[1] = float4(matched.diffuse_albedo, 1);
    outputs[2] = float4(matched.specular_albedo, 1);
    material.pbr.surface.x = 1.5;
    DenoiseGuides dielectric = denoise_surface(ConstantEnergy(), material, float3(0, 0, 1),
                                               float3(0, 0, 1), float4(1, 0, 0, 0));
    outputs[3] = float4(dielectric.diffuse_albedo, 1);
    outputs[4] = float4(dielectric.specular_albedo, 1);
    material.pbr.control.x = 1;
    DenoiseGuides solid = denoise_surface(ConstantEnergy(), material, float3(0, 0, 1),
                                          float3(0, 0, 1), float4(1, 0, 0, 0));
    outputs[5] = float4(solid.diffuse_albedo, 1);
    outputs[6] = float4(solid.specular_albedo, 1);
}

public struct PrefixScene : ITraceScene
{
    public SurfaceMaterial resolve(Hit hit, uint capability, float4 incident)
    {
        SurfaceMaterial result = {};
        result.pbr.base_roughness = float4(1, 1, 1, 0);
        result.pbr.surface.x = 1;
        result.pbr.control.x = 1;
        result.transmitted = incident;
        result.boundary_filter = 1;
        return result;
    }
    public openpbr::Energy energy()
    {
        return { unused_energy, unused_sampler };
    }
    public LightSample sample_sun(geometry::SurfacePoint point, float2 sample)
    {
        return (LightSample)0;
    }
    public LightSample sample_light(geometry::SurfacePoint point, float3 sample)
    {
        return (LightSample)0;
    }
    public float3 light_emission(geometry::SurfacePoint point, LightSample sample, bool solar)
    {
        return sample.emission;
    }
    public float emission_pdf(Hit hit, float3 previous)
    {
        return 0;
    }
    public bool occluded(geometry::Ray ray)
    {
        return false;
    }
    public Environment environment(float3 direction, float3 previous, uint depth)
    {
        return (Environment)0;
    }
    public bool intersect(float3 origin, float3 direction, out Hit hit)
    {
        hit = {};
        hit.surface = { float3(0, 0, origin.z == 0 ? 2 : 4), float3(0, 0, -1), 0 };
        hit.distance = 2;
        hit.identity.x = origin.z == 0 ? 11 : 22;
        return true;
    }
    public void resolve_hit(inout Hit hit, float4 incident)
    {
    }
    public bool prefix(Hit hit)
    {
        return hit.identity.x == 11;
    }
    public void shade(inout TransportCall data)
    {
    }
};

public struct PrefixGuides : IPrefixGuides
{
    public void surface(TransportCall data)
    {
        outputs[0] = float4(data.hit.surface.position, float(data.hit.identity.x));
    }
    public void environment(TransportCall data)
    {
        outputs[0] = -1;
    }
};

[numthreads(64, 1, 1)]
void denoise_prefix_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    TransportCall with_guides = {};
    with_guides.path.direction = float3(0, 0, 1);
    with_guides.path.medium = float4(1, 0, 0, 0);
    with_guides.path.throughput = 1;
    with_guides.path.eta_scale = 1;
    TransportCall without_guides = with_guides;
    Settings settings = { 0, 3, 8, 8 };
    float first, other_first;
    trace_prefix(PrefixScene(), with_guides, settings, first, PrefixGuides());
    trace_prefix(PrefixScene(), without_guides, settings, other_first);
    outputs[1] = float4(with_guides.hit.surface.position, float(with_guides.hit.identity.x));
    outputs[2] = float4(with_guides.path.throughput, first);
    outputs[3] = float4(without_guides.path.throughput, other_first);
    outputs[4] = float4(with_guides.path.direction, float(with_guides.path.depth));
    outputs[5] = float4(without_guides.path.direction, float(without_guides.path.depth));
}
