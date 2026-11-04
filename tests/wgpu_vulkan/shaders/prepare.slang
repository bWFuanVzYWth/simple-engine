module prepare;

struct Parameters
{
    uint frame;
};

[[vk::binding(0, 0)]]
RWTexture2D<float4> input;
[[vk::binding(1, 0)]]
RWTexture2D<float4> output;
[[vk::binding(2, 0)]]
ConstantBuffer<Parameters> parameters;

[numthreads(8, 8, 1)]
void prepare_main(uint3 id: SV_DispatchThreadID)
{
    uint width, height;
    input.GetDimensions(width, height);
    if (id.x >= width || id.y >= height)
        return;
    input[id.xy] = float4(id.x + 1, id.y + 1, parameters.frame, 1);
    output[id.xy] = -1;
}
