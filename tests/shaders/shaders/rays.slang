module rays;

import geometry.ray;

[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(2, 0)]]
RWStructuredBuffer<uint4> outputs;

struct Payload
{
    uint primitive;
    float2 barycentric;
};

[shader("miss")]
void miss_main(inout Payload payload)
{
    payload.primitive = 0xffffffffu;
}

[shader("closesthit")]
void hit_main(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.primitive = PrimitiveIndex();
    payload.barycentric = attributes.barycentrics;
}

bool occluded(geometry::Ray input)
{
    RayDesc ray = { input.origin, input.t_min, input.direction, input.t_max };
    Payload payload = {};
    TraceRay(scene, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER,
             0xff, 0, 1, 0, ray, payload);
    return payload.primitive != 0xffffffffu;
}

[shader("raygeneration")]
void outgoing_main()
{
    uint index = DispatchRaysIndex().x;
    float4 data = inputs[index + 3];
    float3 edge_u = inputs[1].xyz - inputs[0].xyz;
    float3 edge_v = inputs[2].xyz - inputs[0].xyz;
    float3 normal = normalize(cross(edge_u, edge_v));
    float3 point = inputs[0].xyz + data.x * edge_u + data.y * edge_v;
    RayDesc incoming = { point + normal * 10, 0, -normal, 20 };
    Payload payload;
    payload.primitive = 0xffffffffu;
    payload.barycentric = 0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, incoming, payload);
    uint flags = 0;
    if (payload.primitive == 0xffffffffu)
        flags |= 1;
    else
    {
        geometry::SurfacePoint surface = geometry::triangle_point(
            inputs[0].xyz, inputs[1].xyz, inputs[2].xyz, payload.barycentric);
        float3 direction = normalize(normal * 0.01 + normalize(edge_u));
        if (data.w < 0)
            direction = -direction;
        if (occluded(geometry::spawn_ray(surface, direction, 10)))
            flags |= 2;
    }
    outputs[index] = uint4(flags, index + 1, 0, 0);
}

[shader("raygeneration")]
void connection_main()
{
    uint index = DispatchRaysIndex().x;
    float4 data = inputs[index + 3];
    float3 tangent = normalize(inputs[1].xyz - inputs[0].xyz);
    geometry::TriangleEndpoint target =
        geometry::triangle_endpoint(inputs[0].xyz, inputs[1].xyz, inputs[2].xyz, data.xy);
    geometry::SurfacePoint source;
    source.position = target.position + target.normal * 10 + tangent * data.z;
    source.normal = -target.normal;
    source.offset = 0;
    geometry::Ray connection;
    bool valid = geometry::connection_ray(source, target, connection);
    bool blocked = false;
    if (valid)
        blocked = occluded(connection);
    uint flags = valid ? 0 : 1;
    if (blocked != (data.w > 0))
        flags |= 2;
    outputs[index] = uint4(flags, index + 1, blocked ? 1 : 0, valid ? 1 : 0);
}
