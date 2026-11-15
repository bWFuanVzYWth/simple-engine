module trace;

struct Payload
{
    var hit : uint;
};

[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var input : RWTexture2D<float4>;
[[vk::binding(2, 0)]]
var output : RWTexture2D<float4>;

[shader("raygeneration")]
func raygen_main()->void
{
    let pixel : uint2 = DispatchRaysIndex().xy;
    var ray : RayDesc;
    ray.Origin = float3((pixel.x & 1) == 0 ? 0 : 100, 0, -1);
    ray.Direction = float3(0, 0, 1);
    ray.TMin = 0;
    ray.TMax = 2;
    var payload : Payload = {};
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, payload);
    output[pixel] = input[pixel] + float4(payload.hit * 1000, 2000, 3000, 0);
}

[shader("miss")]
func miss_main(inout payload_value: Payload)->void
{
    payload_value.hit = 0;
}

[shader("closesthit")]
func closest_hit_main(inout payload_value: Payload,
                      attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    payload_value.hit = 1;
}
