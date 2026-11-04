module render2d;

struct Instance
{
    float4 rect;
    float4 uv;
    float4 color;
    float4 params;
};

struct View
{
    float4 transform;
    float4 pixel;
};

[[vk::binding(0, 0)]]
StructuredBuffer<Instance> instances;
[[vk::binding(1, 0)]]
ConstantBuffer<View> view;
[[vk::binding(0, 1)]]
Texture2D<float4> image;
[[vk::binding(1, 1)]]
SamplerState image_sampler;

struct VertexOutput
{
    float4 position : SV_Position;
    [[vk::location(0)]]
    float2 uv : TEXCOORD0;
    [[vk::location(1)]]
    float2 local : TEXCOORD1;
    [[vk::location(2)]]
    nointerpolation float2 extent : TEXCOORD2;
    [[vk::location(3)]]
    nointerpolation float4 color : COLOR0;
    [[vk::location(4)]]
    nointerpolation float4 params : TEXCOORD3;
};

[shader("vertex")]
VertexOutput vertex_main(uint vertex_id: SV_VulkanVertexID, uint instance_id: SV_VulkanInstanceID)
{
    const float2 corners[6] = {
        float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
        float2(0.0, 1.0), float2(1.0, 0.0), float2(1.0, 1.0),
    };
    Instance item = instances[instance_id];
    float2 corner = corners[vertex_id];
    float2 margin = item.params.x == 0.0 ? view.pixel.xy : float2(0.0);
    float2 local = corner * (item.rect.zw + 2.0 * margin) - margin;
    float2 position = item.rect.xy + local;
    VertexOutput output;
    output.position = float4(position * view.transform.xy + view.transform.zw, 0.0, 1.0);
    output.uv = item.uv.xy + corner * item.uv.zw;
    output.local = local;
    output.extent = item.rect.zw;
    output.color = item.color;
    output.params = item.params;
    return output;
}

[shader("fragment")]
float4 fragment_main(VertexOutput input) : SV_Target0
{
    float radius = clamp(input.params.y, 0.0, min(input.extent.x, input.extent.y) * 0.5);
    float2 q = abs(input.local - input.extent * 0.5) - input.extent * 0.5 + radius;
    float distance = length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    float width = max(fwidth(distance), 0.00001);
    if (input.params.x == 0.0)
    {
        float coverage = saturate(0.5 - distance / width);
        if (input.params.z > 0.0)
            coverage -= saturate(0.5 - (distance + input.params.z) / width);
        return input.color * coverage;
    }
    // UI 图片和 atlas 固定 LOD 0；不在非一致分支内依赖隐式导数。
    float4 sampled = image.SampleLevel(image_sampler, input.uv, 0.0);
    if (input.params.x == 3.0)
        return input.color * sampled.r;
    return sampled * input.color;
}
