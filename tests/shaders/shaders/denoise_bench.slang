/// 模拟空间 fallback 的存储布局，直接测量生产重建内核。
module denoise_bench;
import denoise.guide;
import denoise.spatial;

struct Frame {
    var extent : uint2;
    var scale : float2;
};

[[vk::binding(0, 0)]]
var parameters : StructuredBuffer<Frame>;
[[vk::binding(1, 0)]]
var input_color : StructuredBuffer<float4>;
[[vk::binding(2, 0)]]
[[vk::image_format("r32f")]]
var depth : RWTexture2D<float>;
[[vk::binding(3, 0)]]
[[vk::image_format("rgba16f")]]
var normal : RWTexture2D<float4>;
[[vk::binding(4, 0)]]
[[vk::image_format("rgba16f")]]
var diffuse : RWTexture2D<float4>;
[[vk::binding(5, 0)]]
[[vk::image_format("rgba16f")]]
var specular : RWTexture2D<float4>;
[[vk::binding(6, 0)]]
var output_color : RWStructuredBuffer<float4>;

struct SpatialInput : denoise::ISpatialImage {
    var width : uint;
    func load(pixel: uint2)->denoise::SpatialSample {
        let color = input_color[pixel.y * width + pixel.x];
        let albedo = diffuse[pixel];
        let flags = uint(round(albedo.w * 255));
        return { color.xyz,
                 depth[pixel],
                 normal[pixel],
                 albedo.xyz,
                 specular[pixel].xyz,
                 (flags & 1u) != 0,
                 color.w >= 0 && (flags & 2u) == 0 };
    }
};

[shader("compute")]
[numthreads(8, 8, 1)]
func spatial_4k_main(id: uint3: SV_DispatchThreadID)->void {
    let frame = parameters[0];
    if (any(id.xy >= frame.extent))
        return;
    let source = fma(float2(id.xy) + 0.5, frame.scale, -0.5);
    var valid : bool;
    let color =
        denoise::spatial_reconstruct(SpatialInput(frame.extent.x), source, frame.extent, valid);
    output_color[id.y * frame.extent.x + id.x] = float4(color, valid ? 1 : -1);
}
