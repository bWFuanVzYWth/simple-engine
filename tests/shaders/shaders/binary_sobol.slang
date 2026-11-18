// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only
module binary_sobol_test;

import sampling.binary_sobol;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

// 每 case 两行：(pixel.x/y, sample, R)、(seed, pair_domain, scalar_domain, 0)。
// 输出的 w 为执行标识，前三项仅存被检验的整数或浮点位模式。
func binary_samples<let Mode : uint>(i: uint)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (i >= rows / 2)
        return;
    let a : uint4 = inputs[2 * i];
    let b : uint4 = inputs[2 * i + 1];
    let sampler : sampling::BinarySobolS8 = sampling::binary_sobol_s8(a.xy, a.z, a.w, b.x);
    if (Mode == 0)
        outputs[i] = uint4(sampler.bits_2d(b.y), sampler.bits_1d(b.z), i + 1);
    else if (Mode == 1)
        outputs[i] = uint4(sampler.bits_3d(b.y, b.z), i + 1);
    else
        outputs[i] = uint4(asuint(sampler.sample_3d(b.y, b.z)), i + 1);
}

[numthreads(64, 1, 1)]
func binary_sobol_bits_main(uint3 id: SV_DispatchThreadID)->void {
    binary_samples<0>(id.x);
}

[numthreads(64, 1, 1)]
func binary_sobol_3d_main(uint3 id: SV_DispatchThreadID)->void {
    binary_samples<1>(id.x);
}

[numthreads(64, 1, 1)]
func binary_sobol_float_main(uint3 id: SV_DispatchThreadID)->void {
    binary_samples<2>(id.x);
}
