// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 未曝光线性 HDR 测光；全部资源为 set0。参数上传须通过 exposure::Frame 校验。
module exposure_compute;
import display.exposure;

struct ExposureParameters
{
    uint2 extent;
    uint linear709;
    uint reset;
    float delta_seconds;
    float compensation;
    uint instant;
    uint padding;
};

[[vk::binding(0, 0)]]
StructuredBuffer<ExposureParameters> parameters;
#if EXPOSURE_TEXTURE
[[vk::binding(1, 0)]]
Texture2D<float4> linear_input;
#else
[[vk::binding(1, 0)]]
StructuredBuffer<float4> linear_input;
#endif
[[vk::binding(2, 0)]]
RWStructuredBuffer<uint> histogram;
[[vk::binding(3, 0)]]
RWStructuredBuffer<display::ExposureState> exposure_state;

groupshared uint bins[256];
groupshared uint valid_count;

[shader("compute")]
[numthreads(16, 16, 1)]
void histogram_main(uint3 group: SV_GroupID, uint3 lane: SV_GroupThreadID,
                    uint index: SV_GroupIndex)
{
    ExposureParameters p = parameters[0];
    bins[index] = 0;
    if (index == 0)
        valid_count = 0;
    GroupMemoryBarrierWithGroupSync();
    uint2 origin = group.xy * 64 + lane.xy * 4;
    uint count = 0;
    for (uint y = 0; y < 4; ++y)
        for (uint x = 0; x < 4; ++x)
        {
            uint2 pixel = origin + uint2(x, y);
            if (any(pixel >= p.extent))
                continue;
#if EXPOSURE_TEXTURE
            float4 value = linear_input.Load(int3(pixel, 0));
#else
            float4 value = linear_input[pixel.y * p.extent.x + pixel.x];
#endif
            if (value.w < 0)
                continue;
            int bin = display::exposure_bin(value.xyz, p.linear709 != 0);
            if (bin < 0)
                continue;
            InterlockedAdd(bins[bin], 1u);
            ++count;
        }
    if (count != 0)
        InterlockedAdd(valid_count, count);
    GroupMemoryBarrierWithGroupSync();
    if (bins[index] != 0)
        InterlockedAdd(histogram[index], bins[index]);
    if (index == 0 && valid_count != 0)
        InterlockedAdd(histogram[256], valid_count);
}

[shader("compute")]
[numthreads(1, 1, 1)]
void update_main()
{
    ExposureParameters p = parameters[0];
    display::ExposureState state = exposure_state[0];
    uint valid = histogram[256];
    bool instant = p.reset != 0 || p.instant != 0 || state.initialized == 0;
    if (valid == 0)
    {
        if (instant)
        {
            state = {};
            state.initialized = 1;
            exposure_state[0] = state;
        }
        return;
    }
    uint begin = valid / 200, end = valid - begin;
    uint cursor = 0, retained = 0;
    float weighted = 0, minimum = 0, maximum = 0;
    for (uint bin = 0; bin < 256; ++bin)
    {
        uint next = cursor + histogram[bin];
        uint low = max(cursor, begin), high = min(next, end);
        if (high > low)
        {
            float logarithm = display::exposure_bin_log(bin);
            if (retained == 0)
                minimum = logarithm;
            maximum = logarithm;
            weighted += logarithm * float(high - low);
            retained += high - low;
        }
        cursor = next;
    }
    if (retained == 0)
        return;
    state.measured_log_brightness = weighted / float(retained);
    state.target_ev = display::exposure_target_ev(state.measured_log_brightness, minimum, maximum,
                                                  p.compensation);
    state.exposure_ev =
        instant ? state.target_ev
                : display::exposure_adapt(state.exposure_ev, state.target_ev, p.delta_seconds);
    state.initialized = 1;
    exposure_state[0] = state;
}
