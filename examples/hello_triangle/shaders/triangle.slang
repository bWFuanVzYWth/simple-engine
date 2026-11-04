module triangle;

struct VertexOutput
{
    float4 position : SV_Position;
    [[vk::location(0)]]
    float3 color : COLOR0;
};

[shader("vertex")]
VertexOutput vertex_main(uint vertex_id: SV_VulkanVertexID)
{
    const float2 positions[3] = {
        float2(-0.6, 0.6),
        float2(0.6, 0.6),
        float2(0.0, -0.6),
    };
    const float3 colors[3] = {
        float3(1.0, 0.0, 0.0),
        float3(0.0, 1.0, 0.0),
        float3(0.0, 0.0, 1.0),
    };

    VertexOutput output;
    output.position = float4(positions[vertex_id], 0.0, 1.0);
    output.color = colors[vertex_id];
    return output;
}

[shader("fragment")]
float4 fragment_main(VertexOutput input) : SV_Target0
{
    return float4(input.color, 1.0);
}
