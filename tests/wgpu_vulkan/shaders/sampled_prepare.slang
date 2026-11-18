// wgpu storage 写入到 native sampled 读取的独立互操作回归。
[[vk::binding(0, 0)]]
[vk::image_format("rgba16f")]
var volume : RWTexture3D<float4>;
[[vk::binding(1, 0)]]
[vk::image_format("rgba32f")]
var plane : RWTexture2D<float4>;
struct Fill {
    var base : float4;
};
[[vk::binding(2, 0)]]
var fill : ConstantBuffer<Fill>;

[shader("compute")]
[numthreads(2, 2, 2)]
func sampled_prepare(uint3 id: SV_DispatchThreadID)->void {
    let base : float = fill.base.x;
    volume[id] = float4(base + float(id.x), 2 * base + float(id.y), 3 * base + float(id.z), 1);
    if (id.z == 0)
        plane[id.xy] = float4(10 * base + float(id.x), 20 * base + float(id.y),
                              30 * base + float(id.x + id.y), 1);
}
