// wgpu storage 写入到 native sampled 读取的独立互操作回归。
#if defined(SAMPLED_PREPARE)
[[vk::binding(0, 0)]]
[vk::image_format("rgba16f")]
RWTexture3D<float4> volume;
[[vk::binding(1, 0)]]
[vk::image_format("rgba32f")]
RWTexture2D<float4> plane;
struct Fill
{
    float4 base;
};
[[vk::binding(2, 0)]]
ConstantBuffer<Fill> fill;

[shader("compute")]
[numthreads(2, 2, 2)]
void sampled_prepare(uint3 id: SV_DispatchThreadID)
{
    float base = fill.base.x;
    volume[id] = float4(base + float(id.x), 2 * base + float(id.y), 3 * base + float(id.z), 1);
    if (id.z == 0)
        plane[id.xy] = float4(10 * base + float(id.x), 20 * base + float(id.y),
                              30 * base + float(id.x + id.y), 1);
}
#else
[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> result;
[[vk::binding(2, 0)]]
Texture3D<float4> volume;
[[vk::binding(3, 0)]]
Texture2D<float4> plane;
[[vk::binding(4, 0)]]
SamplerState linear_clamp;
struct Payload
{
    uint unused;
};

[shader("raygeneration")]
void sampled_raygen()
{
    result[0] = volume.SampleLevel(linear_clamp, float3(.5, .5, .5), 0);
    result[1] = volume.SampleLevel(linear_clamp, float3(-.25, 1.25, .5), 0);
    result[2] = plane.Load(int3(0, 0, 0));
    result[3] = plane.Load(int3(1, 1, 0));
}
[shader("miss")]
void sampled_miss(inout Payload payload)
{
    payload.unused = 0;
}
[shader("closesthit")]
void sampled_closest(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.unused = 0;
}
#endif
