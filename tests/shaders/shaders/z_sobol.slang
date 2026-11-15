module z_sobol_test;

import pt.z_sobol;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

// 每 case 两行：pixel.x/y、sample、R；S、seed、domain、未用。
[numthreads(64, 1, 1)]
func z_sobol_bits_main(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : uint4 = inputs[2 * id.x];
    let b : uint4 = inputs[2 * id.x + 1];
    let sampler : pt::ZSobol = pt::z_sobol(a.xy, a.z, a.w, b.x, b.y);
    let bits : uint2 = sampler.bits_2d(b.z);
    outputs[id.x] = uint4(bits.x, id.x + 1, bits.y, sampler.bits_1d(b.z));
}

[numthreads(64, 1, 1)]
func z_sobol_float_main(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : uint4 = inputs[2 * id.x];
    let b : uint4 = inputs[2 * id.x + 1];
    let sampler : pt::ZSobol = pt::z_sobol(a.xy, a.z, a.w, b.x, b.y);
    let sample : float2 = sampler.sample_2d(b.z);
    outputs[id.x] =
        uint4(asuint(sample.x), id.x + 1, asuint(sample.y), asuint(sampler.sample_1d(b.z)));
}

[numthreads(64, 1, 1)]
func z_sobol_s8_main(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : uint4 = inputs[2 * id.x];
    let b : uint4 = inputs[2 * id.x + 1];
    let sampler : pt::ZSobol = pt::z_sobol_s8(a.xy, a.z, a.w, b.y);
    let bits : uint2 = sampler.bits_2d(b.z);
    outputs[id.x] = uint4(bits.x, id.x + 1, bits.y, sampler.bits_1d(b.z));
}
