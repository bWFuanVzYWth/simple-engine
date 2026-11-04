module readback;

[[vk::binding(0, 0)]]
RWTexture2D<float4> image;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> pixels;

[numthreads(8, 8, 1)]
void readback_main(uint3 id: SV_DispatchThreadID)
{
    uint width, height;
    image.GetDimensions(width, height);
    if (id.x >= width || id.y >= height)
        return;
    pixels[id.y * width + id.x] = image[id.xy];
}
