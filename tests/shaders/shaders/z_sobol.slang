module z_sobol_test;

import pt.z_sobol;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

// 每 case 两行：pixel.x/y、sample、R；S、seed、domain、未用。
[numthreads(64, 1, 1)]
void z_sobol_bits_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    uint4 a = inputs[2 * id.x], b = inputs[2 * id.x + 1];
    pt::ZSobol sampler = pt::z_sobol(a.xy, a.z, a.w, b.x, b.y);
    uint2 bits = sampler.bits_2d(b.z);
    outputs[id.x] = uint4(bits.x, id.x + 1, bits.y, sampler.bits_1d(b.z));
}

[numthreads(64, 1, 1)]
void z_sobol_float_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    uint4 a = inputs[2 * id.x], b = inputs[2 * id.x + 1];
    pt::ZSobol sampler = pt::z_sobol(a.xy, a.z, a.w, b.x, b.y);
    float2 sample = sampler.sample_2d(b.z);
    outputs[id.x] =
        uint4(asuint(sample.x), id.x + 1, asuint(sample.y), asuint(sampler.sample_1d(b.z)));
}

[numthreads(64, 1, 1)]
void z_sobol_s8_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    uint4 a = inputs[2 * id.x], b = inputs[2 * id.x + 1];
    pt::ZSobol sampler = pt::z_sobol_s8(a.xy, a.z, a.w, b.y);
    uint2 bits = sampler.bits_2d(b.z);
    outputs[id.x] = uint4(bits.x, id.x + 1, bits.y, sampler.bits_1d(b.z));
}
