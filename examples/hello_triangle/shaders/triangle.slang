module triangle;

struct VertexOutput {
    var position : float4 : SV_Position;
    [[vk::location(0)]]
    var color : float3 : COLOR0;
};

[shader("vertex")]
func vertex_main(vertex_id: uint: SV_VulkanVertexID)->VertexOutput {
    let positions : float2[3] = {
        float2(-0.6, 0.6),
        float2(0.6, 0.6),
        float2(0.0, -0.6),
    };
    let colors : float3[3] = {
        float3(1.0, 0.0, 0.0),
        float3(0.0, 1.0, 0.0),
        float3(0.0, 0.0, 1.0),
    };

    return { float4(positions[vertex_id], 0.0, 1.0), colors[vertex_id] };
}

[shader("fragment")]
float4 fragment_main(VertexOutput input) : SV_Target0 {
    return float4(input.color, 1.0);
}
