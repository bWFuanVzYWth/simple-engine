module instances;

struct Payload
{
    uint instance;
    uint primitive;
    float distance;
};

[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint> result; // InstanceID、PrimitiveIndex、距离位模式三个 planes。
[[vk::binding(2, 0)]]
StructuredBuffer<float4> rays; // origin/t_min、direction/t_max 两个 planes。

[shader("raygeneration")]
void instance_raygen()
{
    uint index = DispatchRaysIndex().x;
    uint count = DispatchRaysDimensions().x;
    float4 origin = rays[index];
    float4 direction = rays[count + index];
    RayDesc ray = { origin.xyz, origin.w, direction.xyz, direction.w };
    Payload payload = { 0xffffffffu, 0xffffffffu, 0.0 };
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, payload);
    result[index] = payload.instance;
    result[count + index] = payload.primitive;
    result[2 * count + index] = asuint(payload.distance);
}

[shader("miss")]
void instance_miss(inout Payload payload)
{
    payload.instance = 0xffffffffu;
    payload.primitive = 0xffffffffu;
    payload.distance = 0.0;
}

[shader("closesthit")]
void instance_closest(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.instance = InstanceID();
    payload.primitive = PrimitiveIndex();
    payload.distance = RayTCurrent();
}
