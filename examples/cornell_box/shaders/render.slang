module render;

import pt.sampling;
import pt.bsdf;
import pt.transport;
import geometry.ray;
import math.constants;

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

func max_component(value: float3)->float
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
    var ray_descriptor : RayDesc;
    ray_descriptor.Origin = input.origin;
    ray_descriptor.Direction = input.direction;
    ray_descriptor.TMin = input.t_min;
    ray_descriptor.TMax = input.t_max;
    var payload : Payload;
    payload.primitive = 0xffffffffu;
    payload.barycentric = 0.0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray_descriptor, payload);
    hit = (Hit)0;
    if (payload.primitive == 0xffffffffu)
        return false;
    let triangle = triangles[payload.primitive];
    hit.surface =
        triangle_point(triangle.a.xyz, triangle.b.xyz, triangle.c.xyz, payload.barycentric);
    hit.material = uint(triangle.a.w);
    return true;
}

// 场景三角形为 opaque；遮挡只需首个命中，无需 closest-hit 或重建阻挡点。
func occluded(input: Ray)->bool
{
    var ray_descriptor : RayDesc;
    ray_descriptor.Origin = input.origin;
    ray_descriptor.Direction = input.direction;
    ray_descriptor.TMin = input.t_min;
    ray_descriptor.TMax = input.t_max;
    var payload : Payload = {};
    TraceRay(scene, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER,
             0xff, 0, 1, 0, ray_descriptor, payload);
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
    let delta = to - from;
    let distance_squared = dot(delta, delta);
    let cosine = dot(normal, -delta * rsqrt(distance_squared));
    return light_pdf_geometry(distance_squared, cosine, area);
}

func light_pdf_geometry(distance_squared: float, light_cosine: float, area: float)->float
{
    return light_cosine > 0 ? distance_squared / (area * light_cosine) : 0;
}

func direct_light(hit: Hit, material: Material, inout rng_state: uint)->float3
{
    let params = parameters[0];
    let u = random(rng_state);
    let v = random(rng_state);
    let a = params.light_corner.xyz;
    let b = a + params.light_edge_u.xyz;
    let c = b + params.light_edge_v.xyz;
    let d = a + params.light_edge_v.xyz;
    let first_triangle = u >= v;
    let light = triangle_endpoint(a, first_triangle ? b : c, first_triangle ? c : d,
                                  first_triangle ? float2(u - v, v) : float2(u, v - u));
    let delta = light.position - hit.surface.position;
    let distance_squared = dot(delta, delta);
    let direction = delta * rsqrt(distance_squared);
    let cosine = saturate(dot(hit.surface.normal, direction));
    let pdf =
        light_pdf_geometry(distance_squared, dot(light.normal, -direction), params.light_edge_u.w);
    if (cosine <= 0.0 || pdf <= 0.0)
        return 0.0;

    var shadow_ray : Ray;
    if (!connection_ray(hit.surface, light, shadow_ray))
        return 0.0;
    if (occluded(shadow_ray))
        return 0.0;
    let emission = materials[uint(params.light_corner.w)].emission.xyz;
    return lambertian_eval(material.albedo.xyz) * emission *
           (cosine * power_heuristic_over_pdf(pdf, cosine * math::frac_1_pi));
}

// NEE + power heuristic MIS，RR 保留尾部期望；达到安全上限时整张图失败。
// https://pbr-book.org/4ed/Light_Transport_I_Surface_Reflection/A_Better_Path_Tracer
func sample_path(pixel: uint, sample_index: uint)->float4
{
    let params = parameters[0];
    var rng_state = pcg(pcg(pixel ^ pcg(params.options.x)) ^ (sample_index * 0x9e3779b9u));
    let coordinate = uint2(pixel % params.image.x, pixel / params.image.x);
    let film = (float2(coordinate) + float2(random(rng_state), random(rng_state))) *
               params.camera_origin.w;
    let direction =
        normalize(params.camera_forward.xyz + (2.0 * film.x - 1.0) * params.camera_right.xyz +
                  (1.0 - 2.0 * film.y) * params.camera_up.xyz);
    var ray : Ray = { params.camera_origin.xyz, 0.0, direction, 1.0e30 };
    var radiance = float3(0.0);
    var throughput = float3(1.0);
    var previous_position = float3(0.0);
    var previous_pdf = 0.0;

    for (var depth = 0u; depth < params.options.y; ++depth)
    {
        var hit : Hit;
        if (!intersect(ray, hit))
            return float4(radiance, 0.0);
        if (dot(hit.surface.normal, -ray.direction) <= 0.0)
            return float4(radiance, 0.0);
        let material = materials[hit.material];
        if (any(material.emission.xyz > 0.0))
        {
            var weight = 1.0;
            if (depth != 0u)
            {
                weight = power_heuristic(previous_pdf,
                                         light_pdf(previous_position, hit.surface.position,
                                                   hit.surface.normal, params.light_edge_u.w));
            }
            radiance += throughput * material.emission.xyz * weight;
        }
        radiance += throughput * direct_light(hit, material, rng_state);
        throughput *= material.albedo.xyz;
        if (max_component(throughput) == 0.0)
            return float4(radiance, 0.0);
        let u = random(rng_state);
        let v = random(rng_state);
        let bsdf = lambertian_sample(hit.surface.normal, float2(u, v));
        previous_pdf = bsdf.pdf;
        previous_position = hit.surface.position;
        ray = spawn_ray(hit.surface, bsdf.direction, 1.0e30);
        if (depth >= 3u)
        {
            let survival = clamp(max_component(throughput), 0.05, 0.95);
            if (random(rng_state) >= survival)
                return float4(radiance, 0.0);
            throughput /= survival;
        }
    }
    return float4(radiance, 1.0);
}

[shader("raygeneration")]
func render_main()->void
{
    let dispatch_id = DispatchRaysIndex().xy;
    let params = parameters[0];
    if (dispatch_id.x >= params.image.x || dispatch_id.y >= params.image.y)
        return;
    let pixel = dispatch_id.y * params.image.x + dispatch_id.x;
    var sum = params.image.z == 0 ? float4(0.0) : accumulation[pixel];
    if (sum.w != 0.0)
        return;
    for (var sample_offset = 0u; sample_offset < params.image.w; ++sample_offset)
    {
        let value = sample_path(pixel, params.image.z + sample_offset);
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
