// 非 opaque 三角形的候选筛选、closest-hit 资源读取及 RT→compute 可见性。
[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint> result;
[[vk::binding(2, 0)]]
StructuredBuffer<uint> source;

struct Payload
{
    uint value;
};

[shader("raygeneration")]
void native_raygen()
{
    RayDesc ray;
    ray.Origin = float3(.25, .25, -1);
    ray.Direction = float3(0, 0, 1);
    ray.TMin = .001;
    ray.TMax = 4;
    Payload payload;
    payload.value = 0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 0, 0, ray, payload);
    result[0] = payload.value;
}

[shader("anyhit")]
void native_any_hit(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    if (source[PrimitiveIndex()] == 0)
        IgnoreHit();
}

[shader("closesthit")]
void native_closest(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.value = source[PrimitiveIndex()] + 3;
}

[shader("miss")]
void native_miss(inout Payload payload)
{
    payload.value = 17;
}

[shader("compute")]
[numthreads(1, 1, 1)]
void native_post(uint3 id: SV_DispatchThreadID)
{
    result[id.x] = result[id.x] * 2 + 1;
}
