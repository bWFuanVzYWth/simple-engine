// 独立 callable payload、动态 SBT 索引及绑定资源回归；不执行递归或嵌套 CallShader。
[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var source : StructuredBuffer<float4>;
[[vk::binding(3, 0)]]
var tint : Texture2D<float4>;
[[vk::binding(4, 0)]]
var linear_clamp : SamplerState;

struct CallablePayload
{
    var value : float4;
    var lane : uint;
};
struct RayPayload
{
    var unused : uint;
};

[shader("raygeneration")]
func callable_raygen()->void
{
    var lane : uint = DispatchRaysIndex().x;
    var payload : CallablePayload;
    payload.value = float4(float(lane), 1, 2, 1);
    payload.lane = lane;
    let first : uint = lane & 1;
    CallShader(first, payload);
    CallShader(first ^ 1, payload);
    result[lane] = payload.value;
}

[shader("callable")]
func callable_add(inout payload_value: CallablePayload)->void
{
    payload_value.value.xyz +=
        source[payload_value.lane].xyz + tint.SampleLevel(linear_clamp, float2(.5), 0).xyz;
    payload_value.value.w += 1;
}

[shader("callable")]
func callable_scale(inout payload_value: CallablePayload)->void
{
    payload_value.value.xyz *= 2;
    payload_value.value.w += 2;
}

[shader("miss")]
func callable_miss(inout payload_value: RayPayload)->void
{
    payload_value.unused = 0;
}

[shader("closesthit")]
func callable_closest(inout payload_value: RayPayload,
                      attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    payload_value.unused = 0;
}
