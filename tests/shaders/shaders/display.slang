module display_test;

import display.prime_drt;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

display::PrimeDrtParameters parameters(float4 a, float4 b)
{
    return { a.x, a.y, a.z, a.w, b.x, b.y, b.zw };
}

[shader("compute")]
[numthreads(64, 1, 1)]
void display_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    uint i = id.x;
    if (i >= rows / 4)
        return;
    float3 color = asfloat(inputs[4 * i]).xyz;
    display::PrimeDrtParameters p =
        parameters(asfloat(inputs[4 * i + 1]), asfloat(inputs[4 * i + 2]));
    uint mode = inputs[4 * i + 3].y;
    float3 result = mode == 1   ? display::prime_drt_rec2020(color, p)
                    : mode == 2 ? display::prime_drt_linear_bt709(color, p)
                                : display::prime_drt_bt709(color, p);
    outputs[i] = uint4(asuint(result), i + 1);
}
