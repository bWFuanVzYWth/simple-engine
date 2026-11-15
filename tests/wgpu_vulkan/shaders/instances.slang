module instances;

struct Payload
{
    var instance : uint;
    var primitive : uint;
    var distance : float;
};

[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<uint>; // InstanceID、PrimitiveIndex、距离位模式三个 planes。
[[vk::binding(2, 0)]]
var rays : StructuredBuffer<float4>; // origin/t_min、direction/t_max 两个 planes。

[shader("raygeneration")]
func instance_raygen()->void
{
    let index : uint = DispatchRaysIndex().x;
    let count : uint = DispatchRaysDimensions().x;
    let origin : float4 = rays[index];
    let direction : float4 = rays[count + index];
    let ray : RayDesc = { origin.xyz, origin.w, direction.xyz, direction.w };
    var payload : Payload = { 0xffffffffu, 0xffffffffu, 0.0 };
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, payload);
    result[index] = payload.instance;
    result[count + index] = payload.primitive;
    result[2 * count + index] = asuint(payload.distance);
}

[shader("miss")]
func instance_miss(inout payload_value: Payload)->void
{
    payload_value.instance = 0xffffffffu;
    payload_value.primitive = 0xffffffffu;
    payload_value.distance = 0.0;
}

[shader("closesthit")]
func instance_closest(inout payload_value: Payload,
                      attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    payload_value.instance = InstanceID();
    payload_value.primitive = PrimitiveIndex();
    payload_value.distance = RayTCurrent();
}
