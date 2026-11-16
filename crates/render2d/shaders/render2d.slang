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
    var position : float4 : SV_Position;
    [[vk::location(0)]]
    var uv : float2 : TEXCOORD0;
    [[vk::location(1)]]
    var local : float2 : TEXCOORD1;
    [[vk::location(2)]]
    nointerpolation var extent : float2 : TEXCOORD2;
    [[vk::location(3)]]
    nointerpolation var color : float4 : COLOR0;
    [[vk::location(4)]]
    nointerpolation var params : float4 : TEXCOORD3;
};

[shader("vertex")]
func vertex_main(vertex_id: uint: SV_VulkanVertexID, instance_id: uint: SV_VulkanInstanceID)
    ->VertexOutput
{
    let corners : float2[6] = {
        float2(0.0, 0.0), float2(1.0, 0.0), float2(0.0, 1.0),
        float2(0.0, 1.0), float2(1.0, 0.0), float2(1.0, 1.0),
    };
    let instance = instances[instance_id];
    let corner = corners[vertex_id];
    let margin = instance.params.x == 0.0 ? view.pixel.xy : float2(0.0);
    let local = corner * (instance.rect.zw + 2.0 * margin) - margin;
    return {
        float4((instance.rect.xy + local) * view.transform.xy + view.transform.zw, 0.0, 1.0),
        instance.uv.xy + corner * instance.uv.zw,
        local,
        instance.rect.zw,
        instance.color,
        instance.params,
    };
}

[shader("fragment")]
float4 fragment_main(VertexOutput input) : SV_Target0
{
    let radius = clamp(input.params.y, 0.0, min(input.extent.x, input.extent.y) * 0.5);
    let half_extent = input.extent * 0.5;
    let q = abs(input.local - half_extent) - half_extent + radius;
    let signed_distance = length(max(q, float2(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    let filter_width = max(fwidth(signed_distance), 0.00001);
    if (input.params.x == 0.0)
    {
        var coverage = saturate(0.5 - signed_distance / filter_width);
        if (input.params.z > 0.0)
            coverage -= saturate(0.5 - (signed_distance + input.params.z) / filter_width);
        return input.color * coverage;
    }
    // UI 图片和 atlas 固定 LOD 0；不在非一致分支内依赖隐式导数。
    let sampled = image.SampleLevel(image_sampler, input.uv, 0.0);
    if (input.params.x == 3.0)
        return input.color * sampled.r;
    return sampled * input.color;
}
