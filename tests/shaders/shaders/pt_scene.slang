/// 两种命中来源执行同一积分器；不涉及硬件求交或能量表读取。
module pt_scene_test;

import pt.integrator;
import pt_scene_reference;
import geometry.ray;
import openpbr.api;
import openpbr.specialized;
import openpbr.flow;
import light_sampling.sample;
import transport.surface_material;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;
// 两个测试域均不消费 energy；真实 SPIR-V 不应保留这两个 descriptor。
[[vk::binding(2, 0)]]
var unused_energy : Texture3D<float4>;
[[vk::binding(3, 0)]]
var unused_sampler : SamplerState;

using namespace pt;
using namespace geometry;

public struct PaletteScene : IFlowScene {
    public func resolve(hit: Hit, capability: uint, incident: float4)->transport::SurfaceMaterial {
        let base : uint = hit.identity.x;
        let value : float4 = inputs[base];
        let color : float3 = value.xyz * inputs[base + 1].xyz;
        var result : transport::SurfaceMaterial = {};
        result.pbr.base_roughness = float4(color, 0.9);
        result.pbr.surface.x = 1.5;
        result.pbr.control.x = capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC ? 1u : 0u;
        result.transmitted = float4(1, inputs[base + 3].xyz);
        result.emission = color * value.w;
        result.boundary_filter = color;
        return result;
    }
    public func resolve_flow(hit: Hit, flow: uint, incident: float4)->transport::SurfaceMaterial {
        let capability : uint = (flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u
                                    ? openpbr::CAP_DELTA_SOLID_DIELECTRIC
                                    : openpbr::CAP_OPAQUE_DIELECTRIC;
        return resolve(hit, capability, incident);
    }
    public func energy()->openpbr::Energy {
        return { unused_energy, unused_sampler };
    }
    public func sample_sun(point_value: SurfacePoint, sample_value: float2)
        ->light_sampling::LightSample {
        return (light_sampling::LightSample)0;
    }
    public func sample_light(point_value: SurfacePoint, sample_value: float3)
        ->light_sampling::LightSample {
        return (light_sampling::LightSample)0;
    }
    public func light_emission(point_value: SurfacePoint, sample_value: light_sampling::LightSample,
                               solar: bool)
        ->float3 {
        return sample_value.emission;
    }
    public func emission_pdf(hit: Hit, previous: float3)->float {
        return inputs[hit.identity.x + 1].w;
    }
    public func occluded(ray: Ray)->bool {
        return false;
    }
    public func environment(direction: float3, previous: float3, depth: uint)->Environment {
        return (Environment)0;
    }
};

public struct AttributeScene : IScene, IDeltaPrefixScene {
    public func delta_interface(hit: Hit, incident: float4)->DeltaInterface {
        return { hit.surface.normal, float4(1, inputs[hit.identity.y + 2].xyz),
                 hit.texel.xyz * inputs[hit.identity.y].xyz };
    }
    public func resolve(hit: Hit, capability: uint, incident: float4)->transport::SurfaceMaterial {
        let color : float3 = hit.texel.xyz * inputs[hit.identity.y].xyz;
        var result : transport::SurfaceMaterial = {};
        result.pbr.base_roughness = float4(color, 0.9);
        result.pbr.surface.x = 1.5;
        result.pbr.control.x = capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC ? 1u : 0u;
        result.transmitted = float4(1, inputs[hit.identity.y + 2].xyz);
        result.emission = color * hit.texel.w;
        result.boundary_filter = color;
        return result;
    }
    public func energy()->openpbr::Energy {
        return { unused_energy, unused_sampler };
    }
    public func sample_sun(point_value: SurfacePoint, sample_value: float2)
        ->light_sampling::LightSample {
        return (light_sampling::LightSample)0;
    }
    public func sample_light(point_value: SurfacePoint, sample_value: float3)
        ->light_sampling::LightSample {
        return (light_sampling::LightSample)0;
    }
    public func light_emission(point_value: SurfacePoint, sample_value: light_sampling::LightSample,
                               solar: bool)
        ->float3 {
        return sample_value.emission;
    }
    public func emission_pdf(hit: Hit, previous: float3)->float {
        return inputs[hit.identity.y].w;
    }
    public func occluded(ray: Ray)->bool {
        return false;
    }
    public func environment(direction: float3, previous: float3, depth: uint)->Environment {
        return (Environment)0;
    }
};

func store_path(base: uint, path: PathState)->void {
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
func pt_scene_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 4)
        return;
    let base : uint = id.x * 4;
    var palette : TransportCall = {};
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
    var attributes : TransportCall = palette;
    attributes.hit.identity.x = 0xffffffffu;
    attributes.hit.identity.y = base + 1;
    attributes.hit.texel = inputs[base];
    var settings : Settings = { 17, 1, 8, 8, false };
    let mode : uint = asuint(inputs[base + 3].w);
    if (mode == 0) {
        palette.path.depth = 1;
        attributes.path.depth = 1;
        process_flow<1155, PaletteScene>(PaletteScene(), palette, settings);
        process_reference_material<openpbr::CAP_OPAQUE_DIELECTRIC, true, true, AttributeScene>(
            AttributeScene(), attributes, settings);
    } else {
        settings.budget = mode == 1 ? 3 : mode == 2 ? 1 : 64;
        if (mode == 3) {
            palette.path.depth = 64;
            attributes.path.depth = 64;
        }
        process_flow<272, PaletteScene>(PaletteScene(), palette, settings);
        process_delta_prefix(AttributeScene(), attributes, settings);
    }
    store_path(id.x * 14, palette.path);
    store_path(id.x * 14 + 7, attributes.path);
}
