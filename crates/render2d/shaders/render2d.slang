module render2d;

struct Instance
{
    var rect : float4;
    var uv : float4;
    var color : float4;
    var params : float4;
};

struct View
{
    var transform : float4;
    var pixel : float4;
};

[[vk::binding(0, 0)]]
var instances : StructuredBuffer<Instance>;
[[vk::binding(1, 0)]]
var view : ConstantBuffer<View>;
[[vk::binding(0, 1)]]
var image : Texture2D<float4>;
[[vk::binding(1, 1)]]
var image_sampler : SamplerState;

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
func vertex_main(uint vertex_id: SV_VulkanVertexID, uint instance_id: SV_VulkanInstanceID)
    ->VertexOutput
{
    let corners : float2[6] = {
        float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
        float2(0.0, 1.0), float2(1.0, 0.0), float2(1.0, 1.0),
    };
    let item : Instance = instances[instance_id];
    let corner : float2 = corners[vertex_id];
    let margin : float2 = item.params.x == 0.0 ? view.pixel.xy : float2(0.0);
    var local : float2 = corner * (item.rect.zw + 2.0 * margin) - margin;
    var position : float2 = item.rect.xy + local;
    var output : VertexOutput;
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
    let radius : float = clamp(input.params.y, 0.0, min(input.extent.x, input.extent.y) * 0.5);
    let q : float2 = abs(input.local - input.extent * 0.5) - input.extent * 0.5 + radius;
    let distance : float = length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    let width : float = max(fwidth(distance), 0.00001);
    if (input.params.x == 0.0)
    {
        var coverage : float = saturate(0.5 - distance / width);
        if (input.params.z > 0.0)
            coverage -= saturate(0.5 - (distance + input.params.z) / width);
        return input.color * coverage;
    }
    // UI 图片和 atlas 固定 LOD 0；不在非一致分支内依赖隐式导数。
    let sampled : float4 = image.SampleLevel(image_sampler, input.uv, 0.0);
    if (input.params.x == 3.0)
        return input.color * sampled.r;
    return sampled * input.color;
}
