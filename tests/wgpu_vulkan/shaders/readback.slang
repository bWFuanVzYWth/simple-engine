module readback;

[[vk::binding(0, 0)]]
var image : RWTexture2D<float4>;
[[vk::binding(1, 0)]]
var pixels : RWStructuredBuffer<float4>;

[numthreads(8, 8, 1)]
func readback_main(uint3 id: SV_DispatchThreadID)->void
{
    var width : uint;
    var height : uint;
    image.GetDimensions(width, height);
    if (id.x >= width || id.y >= height)
        return;
    pixels[id.y * width + id.x] = image[id.xy];
}
