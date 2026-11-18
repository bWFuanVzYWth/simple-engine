module present;

[[vk::binding(0, 0)]]
var image : Texture2D<float4>;

[shader("vertex")]
float4 vertex_main(uint vertex_id: SV_VulkanVertexID) : SV_Position {
    let corner = float2((vertex_id << 1) & 2, vertex_id & 2);
    return float4(corner * 2 - 1, 0, 1);
}

[shader("fragment")]
float4 fragment_main(float4 position: SV_Position) : SV_Target0 {
    var width : uint;
    var height : uint;
    image.GetDimensions(width, height);
    let value : float4 = image.Load(int3(uint2(position.xy), 0));
    // 缩放离屏验证的哨兵值；红色条纹区分命中与未命中。
    return float4(value.x / (width + 1000), (value.y - 2000) / height, (value.z - 3000) / 64, 1);
}
