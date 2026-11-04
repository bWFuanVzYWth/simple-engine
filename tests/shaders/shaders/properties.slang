module properties;

import pt.sampling;
import pt.bsdf;
import pt.mis;
import geometry.ray;

[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

[numthreads(64, 1, 1)]
void properties_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 4)
        return;
    uint start = id.x * 4;
    float4 a = inputs[start];
    float4 b = inputs[start + 1];
    float3 albedo = inputs[start + 2].xyz;
    uint state = asuint(a.w);
    float random = pt::random(state);
    pt::DirectionSample sample = pt::lambertian_sample(a.xyz, b.xy);
    float cosine = dot(a.xyz, sample.direction);
    float3 throughput = pt::lambertian_eval(albedo) * cosine / sample.pdf;
    float weights = pt::power_weight(b.z, b.w) + pt::power_weight(b.w, b.z);
    uint flags = 0;
    if (!(random >= 0 && random < 1))
        flags |= 1;
    if (!(abs(dot(sample.direction, sample.direction) - 1) < 2e-5))
        flags |= 2;
    if (!(cosine >= -1e-6))
        flags |= 4;
    if (!all(abs(throughput - albedo) < 2e-5))
        flags |= 8;
    if (!(abs(weights - 1) < 2e-6))
        flags |= 16;
    outputs[id.x] = uint4(flags, id.x + 1, asuint(cosine), asuint(random));
}

[numthreads(64, 1, 1)]
void known_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    float4 a = inputs[0];
    float4 b = inputs[1];
    float first = pt::power_weight(a.x, a.y);
    float second = pt::power_weight(a.y, a.x);
    float3 value = pt::lambertian_eval(float3(a.zw, b.x));
    float pdf = pt::lambertian_pdf(b.yzw, inputs[2].xyz);
    uint flags = 0;
    if (!(abs(first - 0.1) < 1e-6 && abs(second - 0.9) < 1e-6))
        flags |= 1;
    if (!all(abs(value - float3(0.07957747154594767, 0.15915494309189535, 0.238732414637843)) <
             1e-6))
        flags |= 2;
    if (!(abs(pdf - 0.15915494309189535) < 1e-6))
        flags |= 4;
    outputs[0] = uint4(flags, 1, asuint(first), asuint(pdf));
}

[numthreads(64, 1, 1)]
void surface_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 12)
        return;
    uint start = id.x * 12;
    float3 v0 = inputs[start].xyz;
    float3 v1 = inputs[start + 1].xyz;
    float3 v2 = inputs[start + 2].xyz;
    float2 bary = inputs[start + 3].xy;
    float4 expected = inputs[start + 4];
    float3 expected_normal = inputs[start + 5].xyz;
    float3x4 object_to_world = float3x4(inputs[start + 6], inputs[start + 7], inputs[start + 8]);
    float3x4 world_to_object = float3x4(inputs[start + 9], inputs[start + 10], inputs[start + 11]);
    geometry::SurfacePoint point =
        geometry::triangle_point(v0, v1, v2, bary, object_to_world, world_to_object);
    float3 front = geometry::spawn_ray(point, point.normal, 10).origin;
    float3 back = geometry::spawn_ray(point, -point.normal, 10).origin;
    float error = max(max(abs(point.position.x - expected.x), abs(point.position.y - expected.y)),
                      abs(point.position.z - expected.z));
    float alignment = dot(point.normal, expected_normal);
    uint flags = 0;
    if (!(error <= expected.w))
        flags |= 1;
    if (!(alignment > 1 - 2e-5 && abs(dot(point.normal, point.normal) - 1) < 2e-5))
        flags |= 2;
    if (!(dot(front - point.position, point.normal) > 0 &&
          dot(back - point.position, point.normal) < 0))
        flags |= 4;
    outputs[id.x] = uint4(flags, id.x + 1, asuint(error), asuint(alignment));
}

float endpoint_retreat(geometry::TriangleEndpoint endpoint, float distance)
{
    geometry::SurfacePoint source;
    source.position = endpoint.position + endpoint.normal * distance;
    source.normal = -endpoint.normal;
    source.offset = 0;
    geometry::Ray ray;
    if (!geometry::connection_ray(source, endpoint, ray))
        return -1;
    precise float3 unshifted = endpoint.position - ray.origin;
    return dot(ray.direction - unshifted, endpoint.normal);
}

[numthreads(64, 1, 1)]
void connection_bound_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    float3x4 object_to_world = float3x4(inputs[4], inputs[5], inputs[6]);
    float3x4 world_to_object = float3x4(inputs[7], inputs[8], inputs[9]);
    geometry::TriangleEndpoint endpoint =
        geometry::triangle_endpoint(inputs[0].xyz, inputs[1].xyz, inputs[2].xyz, inputs[3].xy,
                                    object_to_world, world_to_object);
    outputs[0] =
        uint4(asuint(endpoint.base_offset), asuint(endpoint_retreat(endpoint, inputs[3].z)),
              asuint(endpoint_retreat(endpoint, inputs[3].w)), 1);
}

[numthreads(64, 1, 1)]
void connection_degenerate_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x != 0)
        return;
    geometry::SurfacePoint source;
    source.position = inputs[0].xyz;
    source.normal = inputs[1].xyz;
    source.offset = inputs[0].w;
    geometry::TriangleEndpoint target;
    target.position = inputs[2].xyz;
    target.normal = inputs[3].xyz;
    target.base_offset = inputs[2].w;
    target.traversal_weight = inputs[4].xyz;
    geometry::Ray ray;
    bool coincident = geometry::connection_ray(source, target, ray);
    source.offset = inputs[1].w;
    target.position.z = inputs[3].w;
    target.base_offset = inputs[4].w;
    bool reversed = geometry::connection_ray(source, target, ray);
    outputs[0] = uint4(coincident ? 1 : 0, reversed ? 1 : 0, 0, 1);
}
