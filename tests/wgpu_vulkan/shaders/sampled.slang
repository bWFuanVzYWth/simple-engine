// wgpu storage 写入到 native sampled 读取的独立互操作回归。
[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var volume : Texture3D<float4>;
[[vk::binding(3, 0)]]
var plane : Texture2D<float4>;
[[vk::binding(4, 0)]]
var linear_clamp : SamplerState;
struct Payload
{
    var unused : uint;
};

[shader("raygeneration")]
func sampled_raygen()->void
{
    result[0] = volume.SampleLevel(linear_clamp, float3(.5, .5, .5), 0);
    result[1] = volume.SampleLevel(linear_clamp, float3(-.25, 1.25, .5), 0);
    result[2] = plane.Load(int3(0, 0, 0));
    result[3] = plane.Load(int3(1, 1, 0));
}
[shader("miss")]
func sampled_miss(inout payload_value: Payload)->void
{
    payload_value.unused = 0;
}
[shader("closesthit")]
func sampled_closest(inout payload_value: Payload,
                     attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
    payload_value.unused = 0;
}
