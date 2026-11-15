module rays;

import geometry.ray;

[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var outputs : RWStructuredBuffer<uint4>;

struct Payload
{
    var primitive : uint;
    var barycentric : float2;
};

[shader("miss")]
func miss_main(inout payload_value: Payload)->void
{
    payload_value.primitive = 0xffffffffu;
}

[shader("closesthit")]
func hit_main(inout payload_value: Payload, attributes: BuiltInTriangleIntersectionAttributes)->void
{
    payload_value.primitive = PrimitiveIndex();
    payload_value.barycentric = attributes.barycentrics;
}

func occluded(input: geometry::Ray)->bool
{
    let ray : RayDesc = { input.origin, input.t_min, input.direction, input.t_max };
    var payload : Payload = {};
    TraceRay(scene, RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH | RAY_FLAG_SKIP_CLOSEST_HIT_SHADER,
             0xff, 0, 1, 0, ray, payload);
    return payload.primitive != 0xffffffffu;
}

[shader("raygeneration")]
func outgoing_main()->void
{
    let index : uint = DispatchRaysIndex().x;
    let data : float4 = inputs[index + 3];
    let edge_u : float3 = inputs[1].xyz - inputs[0].xyz;
    let edge_v : float3 = inputs[2].xyz - inputs[0].xyz;
    let normal : float3 = normalize(cross(edge_u, edge_v));
    let point : float3 = inputs[0].xyz + data.x * edge_u + data.y * edge_v;
    let incoming : RayDesc = { point + normal * 10, 0, -normal, 20 };
    var payload : Payload;
    payload.primitive = 0xffffffffu;
    payload.barycentric = 0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, incoming, payload);
    var flags : uint = 0;
    if (payload.primitive == 0xffffffffu)
        flags |= 1;
    else
    {
        let surface : geometry::SurfacePoint = geometry::triangle_point(
                          inputs[0].xyz, inputs[1].xyz, inputs[2].xyz, payload.barycentric);
        var direction : float3 = normalize(normal * 0.01 + normalize(edge_u));
        if (data.w < 0)
            direction = -direction;
        if (occluded(geometry::spawn_ray(surface, direction, 10)))
            flags |= 2;
    }
    outputs[index] = uint4(flags, index + 1, 0, 0);
}

[shader("raygeneration")]
func connection_main()->void
{
    let index : uint = DispatchRaysIndex().x;
    let data : float4 = inputs[index + 3];
    let tangent : float3 = normalize(inputs[1].xyz - inputs[0].xyz);
    let target
        : geometry::TriangleEndpoint =
              geometry::triangle_endpoint(inputs[0].xyz, inputs[1].xyz, inputs[2].xyz, data.xy);
    var source : geometry::SurfacePoint;
    source.position = target.position + target.normal * 10 + tangent * data.z;
    source.normal = -target.normal;
    source.offset = 0;
    var connection : geometry::Ray;
    let valid : bool = geometry::connection_ray(source, target, connection);
    var blocked = false;
    if (valid)
        blocked = occluded(connection);
    var flags : uint = valid ? 0 : 1;
    if (blocked != (data.w > 0))
        flags |= 2;
    outputs[index] = uint4(flags, index + 1, blocked ? 1 : 0, valid ? 1 : 0);
}
