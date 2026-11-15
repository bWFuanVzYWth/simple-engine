// 非 opaque 三角形的候选筛选、closest-hit 资源读取及 RT→compute 可见性。
[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<uint>;
[[vk::binding(2, 0)]]
var source : StructuredBuffer<uint>;

struct Payload
{
    var value : uint;
};

[shader("raygeneration")]
func native_raygen()->void
{
    var ray : RayDesc;
    ray.Origin = float3(.25, .25, -1);
    ray.Direction = float3(0, 0, 1);
    ray.TMin = .001;
    ray.TMax = 4;
    var payload : Payload;
    payload.value = 0;
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 0, 0, ray, payload);
    result[0] = payload.value;
}

[shader("anyhit")]
func native_any_hit(inout payload_value: Payload, attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    if (source[PrimitiveIndex()] == 0)
        IgnoreHit();
}

[shader("closesthit")]
func native_closest(inout payload_value: Payload, attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    payload_value.value = source[PrimitiveIndex()] + 3;
}

[shader("miss")]
func native_miss(inout payload_value: Payload)->void
{
    payload_value.value = 17;
}

[shader("compute")]
[numthreads(1, 1, 1)]
func native_post(uint3 id: SV_DispatchThreadID)->void
{
    result[id.x] = result[id.x] * 2 + 1;
}
