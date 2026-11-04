/// 两种命中来源执行同一积分器；不涉及硬件求交或能量表读取。
module pt_scene_test;

import pt.integrator;
import geometry.ray;
import bsdf.openpbr.api;
import bsdf.openpbr.specialized;
import bsdf.openpbr.flow;

[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;
// 两个测试域均不消费 energy；真实 SPIR-V 不应保留这两个 descriptor。
[[vk::binding(2, 0)]]
Texture3D<float4> unused_energy;
[[vk::binding(3, 0)]]
SamplerState unused_sampler;

using namespace pt;
using namespace geometry;

public struct PaletteScene : IFlowScene
{
    public SurfaceMaterial resolve(Hit hit, uint capability, float4 incident)
    {
        uint base = hit.identity.x;
        float4 value = inputs[base];
        float3 color = value.xyz * inputs[base + 1].xyz;
        SurfaceMaterial result = {};
        result.pbr.base_roughness = float4(color, 0.9);
        result.pbr.surface.x = 1.5;
        result.pbr.control.x = capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC ? 1u : 0u;
        result.transmitted = float4(1, inputs[base + 3].xyz);
        result.emission = color * value.w;
        result.boundary_filter = color;
        return result;
    }
    public SurfaceMaterial resolve_flow(Hit hit, uint flow, float4 incident)
    {
        uint capability = (flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u
                              ? openpbr::CAP_DELTA_SOLID_DIELECTRIC
                              : openpbr::CAP_OPAQUE_DIELECTRIC;
        return resolve(hit, capability, incident);
    }
    public openpbr::Energy energy()
    {
        return { unused_energy, unused_sampler };
    }
    public LightSample sample_sun(SurfacePoint point, float2 sample)
    {
        return (LightSample)0;
    }
    public LightSample sample_light(SurfacePoint point, float3 sample)
    {
        return (LightSample)0;
    }
    public float3 light_emission(SurfacePoint point, LightSample sample, bool solar)
    {
        return sample.emission;
    }
    public float emission_pdf(Hit hit, float3 previous)
    {
        return inputs[hit.identity.x + 1].w;
    }
    public bool occluded(Ray ray)
    {
        return false;
    }
    public Environment environment(float3 direction, float3 previous, uint depth)
    {
        return (Environment)0;
    }
};

public struct AttributeScene : IScene
{
    public SurfaceMaterial resolve(Hit hit, uint capability, float4 incident)
    {
        float3 color = hit.texel.xyz * inputs[hit.identity.y].xyz;
        SurfaceMaterial result = {};
        result.pbr.base_roughness = float4(color, 0.9);
        result.pbr.surface.x = 1.5;
        result.pbr.control.x = capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC ? 1u : 0u;
        result.transmitted = float4(1, inputs[hit.identity.y + 2].xyz);
        result.emission = color * hit.texel.w;
        result.boundary_filter = color;
        return result;
    }
    public openpbr::Energy energy()
    {
        return { unused_energy, unused_sampler };
    }
    public LightSample sample_sun(SurfacePoint point, float2 sample)
    {
        return (LightSample)0;
    }
    public LightSample sample_light(SurfacePoint point, float3 sample)
    {
        return (LightSample)0;
    }
    public float3 light_emission(SurfacePoint point, LightSample sample, bool solar)
    {
        return sample.emission;
    }
    public float emission_pdf(Hit hit, float3 previous)
    {
        return inputs[hit.identity.y].w;
    }
    public bool occluded(Ray ray)
    {
        return false;
    }
    public Environment environment(float3 direction, float3 previous, uint depth)
    {
        return (Environment)0;
    }
};

void store_path(uint base, PathState path)
{
    outputs[base] = asuint(float4(path.throughput, path.previous_pdf));
    outputs[base + 1] = asuint(float4(path.radiance, path.eta_scale));
    outputs[base + 2] = asuint(path.medium);
    outputs[base + 3] = asuint(float4(path.direction, 0));
    outputs[base + 4] = uint4(path.depth, path.dielectric_events, path.status, 1);
    outputs[base + 5] = asuint(float4(path.origin, 0));
    outputs[base + 6] = asuint(float4(path.previous, 0));
}

/// 末顶点 opaque 发光的 Beer／MIS 与匹配 IOR 的 solid 边界采用不同独立用例。
[numthreads(64, 1, 1)]
void pt_scene_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 4)
        return;
    uint base = id.x * 4;
    TransportCall palette = {};
    palette.hit.surface = { float3(0), float3(0, 0, 1), 0 };
    palette.hit.identity.x = base;
    palette.hit.distance = 2;
    palette.hit.front = true;
    palette.path.direction = float3(0, 0, -1);
    palette.path.throughput = 1;
    palette.path.medium = float4(1, inputs[base + 2].xyz);
    palette.path.previous_pdf = 0.25;
    palette.path.origin = float3(3, 5, 7);
    palette.path.previous = float3(11, 13, 17);
    palette.path.eta_scale = 1;
    palette.pixel = uint2(id.x, 0);
    palette.sample = asuint(inputs[base + 2].w);
    TransportCall attributes = palette;
    attributes.hit.identity.x = 0xffffffffu;
    attributes.hit.identity.y = base + 1;
    attributes.hit.texel = inputs[base];
    Settings settings = { 17, 1, 8, 8 };
    if (asuint(inputs[base + 3].w) == 0)
    {
        process_flow<1155, PaletteScene>(PaletteScene(), palette, settings);
        process_material<openpbr::CAP_OPAQUE_DIELECTRIC, true, true, AttributeScene>(
            AttributeScene(), attributes, settings);
    }
    else
    {
        settings.budget = 3;
        process_flow<272, PaletteScene>(PaletteScene(), palette, settings);
        process_material<openpbr::CAP_DELTA_SOLID_DIELECTRIC, false, false, AttributeScene>(
            AttributeScene(), attributes, settings);
    }
    store_path(id.x * 14, palette.path);
    store_path(id.x * 14 + 7, attributes.path);
}
