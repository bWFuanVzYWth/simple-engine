module properties;

import bsdf.lambert;
import geometry.ray;
import sampling.pcg;
import transport.weights;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

[numthreads(64, 1, 1)]
func properties_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 4)
        return;
    let start : uint = id.x * 4;
    let a : float4 = inputs[start];
    let b : float4 = inputs[start + 1];
    let albedo : float3 = inputs[start + 2].xyz;
    var state : uint = asuint(a.w);
    let random : float = sampling::random(state);
    let sample : bsdf::DirectionSample = bsdf::lambertian_sample(a.xyz, b.xy);
    let cosine : float = dot(a.xyz, sample.direction);
    let throughput : float3 = bsdf::lambertian_eval(albedo) * cosine / sample.pdf;
    let weights : float =
                      transport::power_heuristic(b.z, b.w) + transport::power_heuristic(b.w, b.z);
    var flags : uint = 0;
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
func known_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    let a : float4 = inputs[0];
    let b : float4 = inputs[1];
    let first : float = transport::power_heuristic(a.x, a.y);
    let second : float = transport::power_heuristic(a.y, a.x);
    let value : float3 = bsdf::lambertian_eval(float3(a.zw, b.x));
    let pdf : float = bsdf::lambertian_pdf(b.yzw, inputs[2].xyz);
    var flags : uint = 0;
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
func surface_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 12)
        return;
    let start : uint = id.x * 12;
    let v0 : float3 = inputs[start].xyz;
    let v1 : float3 = inputs[start + 1].xyz;
    let v2 : float3 = inputs[start + 2].xyz;
    let bary : float2 = inputs[start + 3].xy;
    let expected : float4 = inputs[start + 4];
    let expected_normal : float3 = inputs[start + 5].xyz;
    let object_to_world = float3x4(inputs[start + 6], inputs[start + 7], inputs[start + 8]);
    let world_to_object = float3x4(inputs[start + 9], inputs[start + 10], inputs[start + 11]);
    let point : geometry::SurfacePoint =
                    geometry::triangle_point(v0, v1, v2, bary, object_to_world, world_to_object);
    let front : float3 = geometry::spawn_ray(point, point.normal, 10).origin;
    let back : float3 = geometry::spawn_ray(point, -point.normal, 10).origin;
    let error : float =
                    max(max(abs(point.position.x - expected.x), abs(point.position.y - expected.y)),
                        abs(point.position.z - expected.z));
    let alignment : float = dot(point.normal, expected_normal);
    var flags : uint = 0;
    if (!(error <= expected.w))
        flags |= 1;
    if (!(alignment > 1 - 2e-5 && abs(dot(point.normal, point.normal) - 1) < 2e-5))
        flags |= 2;
    if (!(dot(front - point.position, point.normal) > 0 &&
          dot(back - point.position, point.normal) < 0))
        flags |= 4;
    outputs[id.x] = uint4(flags, id.x + 1, asuint(error), asuint(alignment));
}

func endpoint_retreat(endpoint: geometry::TriangleEndpoint, distance: float)->float {
    var source : geometry::SurfacePoint;
    source.position = endpoint.position + endpoint.normal * distance;
    source.normal = -endpoint.normal;
    source.offset = 0;
    var ray : geometry::Ray;
    if (!geometry::connection_ray(source, endpoint, ray))
        return -1;
    precise let unshifted : float3 = endpoint.position - ray.origin;
    return dot(ray.direction - unshifted, endpoint.normal);
}

[numthreads(64, 1, 1)]
func connection_bound_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    let object_to_world = float3x4(inputs[4], inputs[5], inputs[6]);
    let world_to_object = float3x4(inputs[7], inputs[8], inputs[9]);
    let endpoint : geometry::TriangleEndpoint =
                       geometry::triangle_endpoint(inputs[0].xyz, inputs[1].xyz, inputs[2].xyz,
                                                   inputs[3].xy, object_to_world, world_to_object);
    outputs[0] =
        uint4(asuint(endpoint.base_offset), asuint(endpoint_retreat(endpoint, inputs[3].z)),
              asuint(endpoint_retreat(endpoint, inputs[3].w)), 1);
}

[numthreads(64, 1, 1)]
func connection_degenerate_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x != 0)
        return;
    var source : geometry::SurfacePoint;
    source.position = inputs[0].xyz;
    source.normal = inputs[1].xyz;
    source.offset = inputs[0].w;
    var target : geometry::TriangleEndpoint;
    target.position = inputs[2].xyz;
    target.normal = inputs[3].xyz;
    target.base_offset = inputs[2].w;
    target.traversal_weight = inputs[4].xyz;
    var ray : geometry::Ray;
    let coincident : bool = geometry::connection_ray(source, target, ray);
    source.offset = inputs[1].w;
    target.position.z = inputs[3].w;
    target.base_offset = inputs[4].w;
    let reversed : bool = geometry::connection_ray(source, target, ray);
    outputs[0] = uint4(coincident ? 1 : 0, reversed ? 1 : 0, 0, 1);
}
