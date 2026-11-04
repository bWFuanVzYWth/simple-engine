module render;

import pt.sampling;
import pt.bsdf;
import pt.mis;
import geometry.ray;

using namespace pt;
using namespace geometry;

struct Triangle
{
    float4 a;
    float4 b;
    float4 c;
};

struct Material
{
    float4 albedo;
    float4 emission;
};

struct Params
{
    uint4 image;   // 宽、高、首个样本索引、批次样本数。
    uint4 options; // 随机种子、安全深度上限、三角形数、材质数。
    float4 camera_origin;
    float4 camera_forward;
    float4 camera_right;
    float4 camera_up;
    float4 light_corner; // w 为光源材质索引。
    float4 light_edge_u;
    float4 light_edge_v;
};

[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
StructuredBuffer<Triangle> triangles;
[[vk::binding(2, 0)]]
StructuredBuffer<Material> materials;
[[vk::binding(3, 0)]]
StructuredBuffer<Params> parameters;
[[vk::binding(4, 0)]]
RWStructuredBuffer<float4> accumulation;

struct Hit
{
    SurfacePoint surface;
    uint material;
};

float maximum(float3 value)
{
    return max(value.x, max(value.y, value.z));
}

struct Payload
{
    uint primitive;
    float2 barycentric;
};

// raygen 内顺序追踪；hit/miss 不再追踪射线，管线调用深度为 1。
// https://docs.vulkan.org/spec/latest/chapters/raytracing.html
bool intersect(Ray input, out Hit hit)
{
    RayDesc ray;
    ray.Origin = input.origin;
    ray.Direction = input.direction;
    ray.TMin = input.t_min;
    ray.TMax = input.t_max;
    Payload payload;
    payload.primitive = 0xffffffffu;
    payload.barycentric = 0.0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, payload);
    hit = (Hit)0;
    if (payload.primitive == 0xffffffffu)
        return false;
    Triangle triangle = triangles[payload.primitive];
    hit.surface =
        triangle_point(triangle.a.xyz, triangle.b.xyz, triangle.c.xyz, payload.barycentric);
    hit.material = uint(triangle.a.w);
    return true;
}

// 场景三角形为 opaque；遮挡只需首个命中，无需 closest-hit 或重建阻挡点。
bool occluded(Ray input)
{
    RayDesc ray;
    ray.Origin = input.origin;
    ray.Direction = input.direction;
    ray.TMin = input.t_min;
    ray.TMax = input.t_max;
    Payload payload = {};
    TraceRay(scene, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER,
             0xff, 0, 1, 0, ray, payload);
    return payload.primitive != 0xffffffffu;
}

[shader("miss")]
void miss_main(inout Payload payload)
{
    payload.primitive = 0xffffffffu;
}

[shader("closesthit")]
void closest_hit_main(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.primitive = PrimitiveIndex();
    payload.barycentric = attributes.barycentrics;
}

// 面积 PDF 转立体角 PDF；只有一个单面矩形光源。
float light_pdf(float3 from, float3 to, float3 normal, float area)
{
    float3 delta = to - from;
    float distance_squared = dot(delta, delta);
    float cosine = dot(normal, -delta * rsqrt(distance_squared));
    return cosine > 0.0 ? distance_squared / (area * cosine) : 0.0;
}

float3 direct_light(Hit hit, Material material, inout uint state)
{
    Params params = parameters[0];
    float u = random(state);
    float v = random(state);
    float3 a = params.light_corner.xyz;
    float3 b = a + params.light_edge_u.xyz;
    float3 c = b + params.light_edge_v.xyz;
    float3 d = a + params.light_edge_v.xyz;
    bool first = u >= v;
    TriangleEndpoint light = triangle_endpoint(a, first ? b : c, first ? c : d,
                                               first ? float2(u - v, v) : float2(u, v - u));
    float3 cross_edges = cross(params.light_edge_u.xyz, params.light_edge_v.xyz);
    float area = length(cross_edges);
    float3 direction = normalize(light.position - hit.surface.position);
    float cosine = dot(hit.surface.normal, direction);
    float pdf = light_pdf(hit.surface.position, light.position, light.normal, area);
    if (cosine <= 0.0 || pdf <= 0.0)
        return 0.0;

    Ray connection;
    if (!connection_ray(hit.surface, light, connection))
        return 0.0;
    if (occluded(connection))
        return 0.0;
    float3 emission = materials[uint(params.light_corner.w)].emission.xyz;
    return lambertian_eval(material.albedo.xyz) * emission *
           (cosine * power_weight(pdf, lambertian_pdf(hit.surface.normal, direction)) / pdf);
}

// NEE + power heuristic MIS，RR 保留尾部期望；达到安全上限时整张图失败。
// https://pbr-book.org/4ed/Light_Transport_I_Surface_Reflection/A_Better_Path_Tracer
float4 sample_path(uint pixel, uint sample)
{
    Params params = parameters[0];
    uint state = pcg(pcg(pixel ^ pcg(params.options.x)) ^ (sample * 0x9e3779b9u));
    uint2 coordinate = uint2(pixel % params.image.x, pixel / params.image.x);
    float2 film =
        (float2(coordinate) + float2(random(state), random(state))) / float2(params.image.xy);
    float3 direction =
        normalize(params.camera_forward.xyz + (2.0 * film.x - 1.0) * params.camera_right.xyz +
                  (1.0 - 2.0 * film.y) * params.camera_up.xyz);
    Ray ray = { params.camera_origin.xyz, 0.0, direction, 1.0e30 };
    float3 radiance = 0.0;
    float3 throughput = 1.0;
    float3 previous_position = 0.0;
    float previous_pdf = 0.0;

    for (uint depth = 0; depth < params.options.y; ++depth)
    {
        Hit hit;
        if (!intersect(ray, hit))
            return float4(radiance, 0.0);
        if (dot(hit.surface.normal, -ray.direction) <= 0.0)
            return float4(radiance, 0.0);
        Material material = materials[hit.material];
        if (any(material.emission.xyz > 0.0))
        {
            float weight = 1.0;
            if (depth != 0)
            {
                float area = length(cross(params.light_edge_u.xyz, params.light_edge_v.xyz));
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
        float u = random(state);
        float v = random(state);
        DirectionSample bsdf = lambertian_sample(hit.surface.normal, float2(u, v));
        previous_pdf = bsdf.pdf;
        previous_position = hit.surface.position;
        ray = spawn_ray(hit.surface, bsdf.direction, 1.0e30);
        if (depth >= 3)
        {
            float survival = clamp(maximum(throughput), 0.05, 0.95);
            if (random(state) >= survival)
                return float4(radiance, 0.0);
            throughput /= survival;
        }
    }
    return float4(radiance, 1.0);
}

[shader("raygeneration")]
void render_main()
{
    uint2 dispatch_id = DispatchRaysIndex().xy;
    Params params = parameters[0];
    if (dispatch_id.x >= params.image.x || dispatch_id.y >= params.image.y)
        return;
    uint pixel = dispatch_id.y * params.image.x + dispatch_id.x;
    float4 sum = params.image.z == 0 ? float4(0.0) : accumulation[pixel];
    if (sum.w != 0.0)
        return;
    for (uint sample = 0; sample < params.image.w; ++sample)
    {
        float4 value = sample_path(pixel, params.image.z + sample);
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
