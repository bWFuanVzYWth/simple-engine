/// 规范 guide、PSR 和空间核的实际 GPU 行为；场景为解析几何，不依赖应用策略。
module reconstruction_test;
import pt.reconstruction;
import pt.spatial_denoise;
import bsdf.openpbr.specialized;
import bsdf.openpbr.api;
import pt.scene;
import geometry.ray;
import pt.key_guides;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var unused_energy : Texture3D<float4>;
[[vk::binding(3, 0)]]
var unused_sampler : SamplerState;
using namespace pt;

func camera(x: float)->DenoiseCamera
{
    return { float4(x, 0, 0, 1), float4(0, 0, 1, 2), float4(1, 0, 0, 0), float4(0, 1, 0, 0) };
}

[numthreads(64, 1, 1)]
func psr_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x != 0)
        return;
    let mode = uint(inputs[0].x);
    var state : Psr = psr_initial();
    var position = float3(0, 0, -2);
    var normal = float3(0, 0, 1);
    if (mode != 2)
        state = psr_append(state, float3(0, 0, 2), float3(0, 0, -1), true, false, true);
    if (mode == 1)
    {
        state = psr_append(state, float3(0, 0, -2), float3(0, 0, 1), true, false, true);
        position = float3(0, 0, 1);
    }
    if (mode == 2)
    {
        state = psr_append(state, float3(0, 0, 2), float3(0, 0, -1), false, true, false);
        position = float3(3, 0, 6);
        normal = normalize(float3(1, 0, -1));
    }
    var virtual_position : float3;
    var virtual_normal : float3;
    let valid : bool = psr_surface(state, 0, float3(0, 0, 1), position, normal, normal,
                                   virtual_position, virtual_normal);
    outputs[0] = float4(virtual_position, float(valid));
    outputs[1] = float4(virtual_normal, float(state.odd));
    outputs[2] = float4(float(state.count), float(state.refracted), float(state.fixed), 0);
}

public struct GuideScene : IGuideScene, IKeyGuideTrace
{
    public var mode : uint;
    public func guide_material(hit: Hit, incident: float4)->SurfaceMaterial
    {
        var result : SurfaceMaterial = {};
        result.boundary_filter = 0;
        result.transmitted = float4(mode == 2 ? 1 : 1.5, 100, 100, 100);
        result.pbr.surface.x = 1.5;
        if (hit.capability == openpbr::CAP_DELTA_CONDUCTOR)
            result.pbr.surface.y = 1;
        else if (hit.capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC)
            result.pbr.control.x = 1;
        else
        {
            result.pbr.base_roughness = float4(1, 1, 1, 0.5);
            result.pbr.surface.x = 1;
        }
        return result;
    }
    public func energy()->openpbr::Energy
    {
        return { unused_energy, unused_sampler };
    }
    public func intersect(origin: float3, direction: float3, out hit: Hit)->bool
    {
        hit = {};
        if (mode == 4)
            return false;
        hit.surface = { origin + direction * 4, float3(0, 0, -1), 0 };
        hit.distance = 4;
        hit.capability = openpbr::CAP_OPAQUE_DIELECTRIC;
        hit.identity.x = 22;
        return true;
    }
    public func resolve_hit(inout hit: Hit, incident: float4)->void
    {
    }
    public func guide_pose(hit: Hit)->GuidePose
    {
        return { hit.surface.position, true, true };
    }
    public func key_vertex(hit: Hit, direction: float3, incident: float4)->KeyGuideVertex
    {
        let material : SurfaceMaterial = guide_material(hit, incident);
        if (hit.capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC)
            return make_key_guide<338>(hit, material, energy(), guide_pose(hit), direction,
                                       incident);
        if (hit.capability == openpbr::CAP_DELTA_CONDUCTOR)
            return make_key_guide<68>(hit, material, energy(), guide_pose(hit), direction,
                                      incident);
        return make_key_guide<131>(hit, material, energy(), guide_pose(hit), direction, incident);
    }
    public func intersect_guide(origin: float3, direction: float3, incident: float4,
                                out vertex: KeyGuideVertex)
        ->bool
    {
        var direction = direction;
        vertex = {};
        var hit : Hit;
        if (!intersect(origin, direction, hit))
            return false;
        vertex = key_vertex(hit, direction, incident);
        return true;
    }
};

[numthreads(64, 1, 1)]
func guide_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x != 0)
        return;
    let mode = uint(inputs[0].x);
    var first : TransportCall = {};
    first.hit.surface = { float3(0, 0, 2), float3(0, 0, -1), 0 };
    first.hit.capability = mode == 1 || mode == 2 ? openpbr::CAP_DELTA_SOLID_DIELECTRIC
                           : mode == 6            ? openpbr::CAP_OPAQUE_DIELECTRIC
                                                  : openpbr::CAP_DELTA_CONDUCTOR;
    first.hit.identity.x = 11;
    first.hit.distance = 2;
    first.path.direction = mode == 2 ? normalize(float3(0.9, 0, 0.4358899)) : float3(0, 0, 1);
    first.path.medium = float4(mode == 2 ? 1.5 : 1, 100, 100, 100);
    // 照明已经零贡献终止；guide 仍须独立抵达终点。
    first.path.status = PATH_DONE;
    first.path.radiance = float3(3, 5, 7);
    let scene : GuideScene = { mode };
    let endpoint : GuideEndpoint = mode == 6 ? trace_reflection_guide(scene, first, 2)
                                             : trace_guide(scene, first, mode == 5 ? 1 : 2);
    let uv : float2 = 0.5;
    let projection : GuideProjection =
                         project_guide(endpoint, camera(0), camera(-1), uv, uint2(800, 400), true);
    outputs[0] = float4(endpoint.hit.surface.position, float(endpoint.hit.identity.x));
    outputs[1] = float4(endpoint.direction, float(endpoint.queries));
    outputs[2] = float4(projection.depth, projection.motion, float(projection.flags));
    outputs[3] = float4(first.path.radiance, float(first.path.status));
    outputs[4] = float4(float(endpoint.psr.odd), float(endpoint.psr.refracted),
                        float(endpoint.use_interface_specular), endpoint.distance);
}

[numthreads(64, 1, 1)]
func keyed_guide_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x != 0)
        return;
    let mode = uint(inputs[0].x);
    var hit : Hit = {};
    hit.surface = { float3(0, 0, 2), float3(0, 0, -1), 0 };
    hit.capability = mode == 1 || mode == 2 ? openpbr::CAP_DELTA_SOLID_DIELECTRIC
                     : mode >= 6            ? openpbr::CAP_OPAQUE_DIELECTRIC
                                            : openpbr::CAP_DELTA_CONDUCTOR;
    hit.distance = 2;
    let direction : float3 = mode == 2 ? normalize(float3(0.9, 0, 0.4358899)) : float3(0, 0, 1);
    let incident = float4(mode == 2 ? 1.5 : 1, 100, 100, 100);
    let scene : GuideScene = { mode };
    let vertex : KeyGuideVertex = scene.key_vertex(hit, direction, incident);
    let keyed : KeyGuideEndpoint = trace_key_guide(scene, vertex, float3(0), direction, incident,
                                                   mode == 5 ? 1 : 2, mode >= 6, mode == 7);
    let endpoint : GuideEndpoint = keyed.endpoint;
    let projection : GuideProjection = project_guide(endpoint, camera(0), camera(-1), float2(0.5),
                                                     uint2(800, 400), true);
    outputs[0] = float4(endpoint.hit.surface.position, 0);
    outputs[1] = float4(endpoint.direction, float(endpoint.queries));
    outputs[2] = float4(projection.depth, projection.motion, float(projection.flags));
    outputs[3] = float4(3, 5, 7, float(PATH_DONE));
    outputs[4] = float4(float(endpoint.psr.odd), float(endpoint.psr.refracted),
                        float(endpoint.use_interface_specular), endpoint.distance);
}

[numthreads(64, 1, 1)]
func reflection_motion_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x != 0)
        return;
    var motion : float2;
    let mode = uint(inputs[0].x);
    let valid : bool = reflection_distance_motion(
                    camera(0), camera(-1), float2(0.5), mode == 2 ? DENOISE_SKY_DEPTH : 10,
                    mode == 1 ? 0 : (mode == 3 ? -1 : 10), float2(7, 3), uint2(800, 400), motion);
    outputs[0] = float4(motion, float(valid), 0);
}

[numthreads(64, 1, 1)]
func rigid_motion_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x != 0)
        return;
    let mode = uint(inputs[0].x);
    let previous = float3x4(0, -1, 0, 3, 1, 0, 0, -2, 0, 0, 1, 4);
    let rigid : float3 = rigid_previous_position(float3(1, 0, 0), float3(2, 0, 0), float3(1, 2, 0),
                                                 float2(0.25, 0.5), previous);
    outputs[0] = float4(rigid, 1);
    var endpoint : GuideEndpoint = {};
    endpoint.hit.surface = { float3(0, 0, 10), float3(0, 0, -1), 0 };
    endpoint.camera_direction = endpoint.direction = float3(0, 0, 1);
    endpoint.pose = { float3(1, 0, 10), mode == 0, true };
    endpoint.psr = psr_initial();
    if (mode == 2)
    {
        endpoint.pose.known = true;
        endpoint.psr.fixed = false;
    }
    let projection : GuideProjection = project_guide(endpoint, camera(0), camera(0), float2(0.5),
                                                     uint2(800, 400), true);
    outputs[1] = float4(projection.motion, projection.depth, float(projection.flags));
}

public struct SpatialImage : ISpatialImage
{
    public var mode : uint;
    public func load(pixel: uint2)->SpatialSample
    {
        let color : float3 = mode == 0 ? float3(2, 3, 4)
                                       : (pixel.x == 1 ? float3(2, 0, 0) : float3(0, 0, 2));
        let depth : float = mode == 1 && pixel.x != 1 ? 100 : 10;
        let diffuse : float3 = mode == 2 && pixel.x != 1 ? float3(1) : float3(0);
        return { color, depth, float4(0, 0, -1, 0.5), diffuse, float3(0), true, true };
    }
};

[numthreads(64, 1, 1)]
func spatial_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x != 0)
        return;
    let image : SpatialImage = { uint(inputs[0].x) };
    var valid : bool;
    let color : float3 = spatial_reconstruct(image, float2(1, 0), uint2(3, 1), valid);
    outputs[0] = float4(color, float(valid));
}
