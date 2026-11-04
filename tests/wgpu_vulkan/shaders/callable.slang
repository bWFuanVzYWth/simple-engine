// 独立 callable payload、动态 SBT 索引及绑定资源回归；不执行递归或嵌套 CallShader。
[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> result;
[[vk::binding(2, 0)]]
StructuredBuffer<float4> source;
[[vk::binding(3, 0)]]
Texture2D<float4> tint;
[[vk::binding(4, 0)]]
SamplerState linear_clamp;

struct CallablePayload
{
    float4 value;
    uint lane;
};
struct RayPayload
{
    uint unused;
};

[shader("raygeneration")]
void callable_raygen()
{
    uint lane = DispatchRaysIndex().x;
    CallablePayload payload;
    payload.value = float4(float(lane), 1, 2, 1);
    payload.lane = lane;
    uint first = lane & 1;
    CallShader(first, payload);
    CallShader(first ^ 1, payload);
    result[lane] = payload.value;
}

[shader("callable")]
void callable_add(inout CallablePayload payload)
{
    payload.value.xyz +=
        source[payload.lane].xyz + tint.SampleLevel(linear_clamp, float2(.5), 0).xyz;
    payload.value.w += 1;
}

[shader("callable")]
void callable_scale(inout CallablePayload payload)
{
    payload.value.xyz *= 2;
    payload.value.w += 2;
}

[shader("miss")]
void callable_miss(inout RayPayload payload)
{
    payload.unused = 0;
}

[shader("closesthit")]
void callable_closest(inout RayPayload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.unused = 0;
}
