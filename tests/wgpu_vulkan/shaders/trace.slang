module trace;

struct Payload
{
    uint hit;
};

[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
RWTexture2D<float4> input;
[[vk::binding(2, 0)]]
RWTexture2D<float4> output;

[shader("raygeneration")]
void raygen_main()
{
    uint2 pixel = DispatchRaysIndex().xy;
    RayDesc ray;
    ray.Origin = float3((pixel.x & 1) == 0 ? 0 : 100, 0, -1);
    ray.Direction = float3(0, 0, 1);
    ray.TMin = 0;
    ray.TMax = 2;
    Payload payload = {};
    TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, payload);
    output[pixel] = input[pixel] + float4(payload.hit * 1000, 2000, 3000, 0);
}

[shader("miss")]
void miss_main(inout Payload payload)
{
    payload.hit = 0;
}

[shader("closesthit")]
void closest_hit_main(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.hit = 1;
}
