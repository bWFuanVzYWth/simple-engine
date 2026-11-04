// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only
module binary_sobol_test;

import pt.binary_sobol;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

// 每 case 两行：(pixel.x/y, sample, R)、(seed, pair_domain, scalar_domain, 0)。
// 输出的 w 为执行标识，前三项仅存被检验的整数或浮点位模式。
void binary_samples<let Mode : uint>(uint i)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (i >= rows / 2)
        return;
    uint4 a = inputs[2 * i], b = inputs[2 * i + 1];
    pt::BinarySobolS8 sampler = pt::binary_sobol_s8(a.xy, a.z, a.w, b.x);
    if (Mode == 0)
        outputs[i] = uint4(sampler.bits_2d(b.y), sampler.bits_1d(b.z), i + 1);
    else if (Mode == 1)
        outputs[i] = uint4(sampler.bits_3d(b.y, b.z), i + 1);
    else
        outputs[i] = uint4(asuint(sampler.sample_3d(b.y, b.z)), i + 1);
}

[numthreads(64, 1, 1)]
void binary_sobol_bits_main(uint3 id: SV_DispatchThreadID)
{
    binary_samples<0>(id.x);
}

[numthreads(64, 1, 1)]
void binary_sobol_3d_main(uint3 id: SV_DispatchThreadID)
{
    binary_samples<1>(id.x);
}

[numthreads(64, 1, 1)]
void binary_sobol_float_main(uint3 id: SV_DispatchThreadID)
{
    binary_samples<2>(id.x);
}
