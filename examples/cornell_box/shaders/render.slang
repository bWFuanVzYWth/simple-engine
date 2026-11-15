module render;

import pt.sampling;
import pt.bsdf;
import pt.mis;
import pt.transport;
import geometry.ray;

using namespace pt;
using namespace geometry;

struct Triangle
{
    var a : float4;
    var b : float4;
    var c : float4;
};

struct Material
{
    var albedo : float4;
    var emission : float4;
};

struct Params
{
    var image : uint4;          // 宽、高、首个样本索引、批次样本数。
    var options : uint4;        // 随机种子、安全深度上限、三角形数、材质数。
    var camera_origin : float4; // w为输入边长的倒数。
    var camera_forward : float4;
    var camera_right : float4;
    var camera_up : float4;
    var light_corner : float4; // w 为光源材质索引。
    var light_edge_u : float4; // w为主机计算的矩形面积。
    var light_edge_v : float4;
};

[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var triangles : StructuredBuffer<Triangle>;
[[vk::binding(2, 0)]]
var materials : StructuredBuffer<Material>;
[[vk::binding(3, 0)]]
var parameters : StructuredBuffer<Params>;
[[vk::binding(4, 0)]]
var accumulation : RWStructuredBuffer<float4>;

struct Hit
{
    var surface : SurfacePoint;
    var material : uint;
};

func maximum(value: float3)->float
{
    return max(value.x, max(value.y, value.z));
}

struct Payload
{
    var primitive : uint;
    var barycentric : float2;
};

// raygen 内顺序追踪；hit/miss 不再追踪射线，管线调用深度为 1。
// https://docs.vulkan.org/spec/latest/chapters/raytracing.html
func intersect(input: Ray, out hit: Hit)->bool
{
    var ray : RayDesc;
    ray.Origin = input.origin;
    ray.Direction = input.direction;
    ray.TMin = input.t_min;
    ray.TMax = input.t_max;
    var payload : Payload;
    payload.primitive = 0xffffffffu;
    payload.barycentric = 0.0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, payload);
    hit = (Hit)0;
    if (payload.primitive == 0xffffffffu)
        return false;
    let triangle : Triangle = triangles[payload.primitive];
    hit.surface =
        triangle_point(triangle.a.xyz, triangle.b.xyz, triangle.c.xyz, payload.barycentric);
    hit.material = uint(triangle.a.w);
    return true;
}

// 场景三角形为 opaque；遮挡只需首个命中，无需 closest-hit 或重建阻挡点。
func occluded(input: Ray)->bool
{
    var ray : RayDesc;
    ray.Origin = input.origin;
    ray.Direction = input.direction;
    ray.TMin = input.t_min;
    ray.TMax = input.t_max;
    var payload : Payload = {};
    TraceRay(scene, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER,
             0xff, 0, 1, 0, ray, payload);
    return payload.primitive != 0xffffffffu;
}

[shader("miss")]
func miss_main(inout payload_value: Payload)->void
{
    payload_value.primitive = 0xffffffffu;
}

[shader("closesthit")]
func closest_hit_main(inout payload_value: Payload,
                      attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    payload_value.primitive = PrimitiveIndex();
    payload_value.barycentric = attributes.barycentrics;
}

// 面积 PDF 转立体角 PDF；只有一个单面矩形光源。
func light_pdf(from: float3, to: float3, normal: float3, area: float)->float
{
    let delta : float3 = to - from;
    let distance_squared : float = dot(delta, delta);
    let cosine : float = dot(normal, -delta * rsqrt(distance_squared));
    return light_pdf_geometry(distance_squared, cosine, area);
}

func light_pdf_geometry(distance_squared: float, light_cosine: float, area: float)->float
{
    return light_cosine > 0 ? distance_squared / (area * light_cosine) : 0;
}

func direct_light(hit: Hit, material: Material, inout state: uint)->float3
{
    let params : Params = parameters[0];
    let u : float = random(state);
    let v : float = random(state);
    let a : float3 = params.light_corner.xyz;
    let b : float3 = a + params.light_edge_u.xyz;
    let c : float3 = b + params.light_edge_v.xyz;
    let d : float3 = a + params.light_edge_v.xyz;
    let first : bool = u >= v;
    let light : TriangleEndpoint = triangle_endpoint(a, first ? b : c, first ? c : d,
                                                     first ? float2(u - v, v) : float2(u, v - u));
    let delta = light.position - hit.surface.position;
    let squared = dot(delta, delta);
    let direction = delta * rsqrt(squared);
    let cosine = saturate(dot(hit.surface.normal, direction));
    let pdf = light_pdf_geometry(squared, dot(light.normal, -direction), params.light_edge_u.w);
    if (cosine <= 0.0 || pdf <= 0.0)
        return 0.0;

    var connection : Ray;
    if (!connection_ray(hit.surface, light, connection))
        return 0.0;
    if (occluded(connection))
        return 0.0;
    let emission : float3 = materials[uint(params.light_corner.w)].emission.xyz;
    return lambertian_eval(material.albedo.xyz) * emission *
           (cosine * power_heuristic_over_pdf(pdf, cosine * (1.0 / pi)));
}

// NEE + power heuristic MIS，RR 保留尾部期望；达到安全上限时整张图失败。
// https://pbr-book.org/4ed/Light_Transport_I_Surface_Reflection/A_Better_Path_Tracer
func sample_path(pixel: uint, sample_value: uint)->float4
{
    let params : Params = parameters[0];
    var state : uint = pcg(pcg(pixel ^ pcg(params.options.x)) ^ (sample_value * 0x9e3779b9u));
    let coordinate = uint2(pixel % params.image.x, pixel / params.image.x);
    let film = (float2(coordinate) + float2(random(state), random(state))) * params.camera_origin.w;
    let direction : float3 = normalize(params.camera_forward.xyz +
                                       (2.0 * film.x - 1.0) * params.camera_right.xyz +
                                       (1.0 - 2.0 * film.y) * params.camera_up.xyz);
    var ray : Ray = { params.camera_origin.xyz, 0.0, direction, 1.0e30 };
    var radiance : float3 = 0.0;
    var throughput : float3 = 1.0;
    var previous_position : float3 = 0.0;
    var previous_pdf = 0.0;

    for (var depth: uint = 0; depth < params.options.y; ++depth)
    {
        var hit : Hit;
        if (!intersect(ray, hit))
            return float4(radiance, 0.0);
        if (dot(hit.surface.normal, -ray.direction) <= 0.0)
            return float4(radiance, 0.0);
        let material : Material = materials[hit.material];
        if (any(material.emission.xyz > 0.0))
        {
            var weight = 1.0;
            if (depth != 0)
            {
                let area = params.light_edge_u.w;
                weight =
                    power_weight(previous_pdf, light_pdf(previous_position, hit.surface.position,
                                                         hit.surface.normal, area));
            }
            radiance += throughput * material.emission.xyz * weight;
        }
        radiance += throughput * direct_light(hit, material, state);
        throughput *= material.albedo.xyz;
        if (maximum(throughput) == 0.0)
            return float4(radiance, 0.0);
        let u : float = random(state);
        let v : float = random(state);
        let bsdf : DirectionSample = lambertian_sample(hit.surface.normal, float2(u, v));
        previous_pdf = bsdf.pdf;
        previous_position = hit.surface.position;
        ray = spawn_ray(hit.surface, bsdf.direction, 1.0e30);
        if (depth >= 3)
        {
            let survival : float = clamp(maximum(throughput), 0.05, 0.95);
            if (random(state) >= survival)
                return float4(radiance, 0.0);
            throughput /= survival;
        }
    }
    return float4(radiance, 1.0);
}

[shader("raygeneration")]
func render_main()->void
{
    let dispatch_id : uint2 = DispatchRaysIndex().xy;
    let params : Params = parameters[0];
    if (dispatch_id.x >= params.image.x || dispatch_id.y >= params.image.y)
        return;
    let pixel : uint = dispatch_id.y * params.image.x + dispatch_id.x;
    var sum : float4 = params.image.z == 0 ? float4(0.0) : accumulation[pixel];
    if (sum.w != 0.0)
        return;
    for (var sample: uint = 0; sample < params.image.w; ++sample)
    {
        let value : float4 = sample_path(pixel, params.image.z + sample);
        if (value.w != 0.0)
        {
            sum.w = value.w;
            break;
        }
        // 保持跨 dispatch 的逐样本累加顺序。
        sum.xyz += value.xyz;
        if (!isfinite(sum.x) || !isfinite(sum.y) || !isfinite(sum.z))
        {
            sum.w = 2.0;
            break;
        }
    }
    accumulation[pixel] = sum;
}
