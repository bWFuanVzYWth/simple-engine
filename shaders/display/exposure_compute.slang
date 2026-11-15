// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 未曝光线性 HDR 测光；全部资源为 set0。参数上传须通过 exposure::Frame 校验。
module exposure_compute;
import display.exposure;

struct ExposureParameters
{
    var extent : uint2;
    var linear709 : uint;
    var reset : uint;
    var delta_seconds : float;
    var compensation : float;
    var instant : uint;
    var padding : uint;
};

[[vk::binding(0, 0)]]
var parameters : StructuredBuffer<ExposureParameters>;
[[vk::binding(2, 0)]]
var histogram : RWStructuredBuffer<uint>;
[[vk::binding(3, 0)]]
var exposure_state : RWStructuredBuffer<display::ExposureState>;

groupshared var bins : uint[256];
groupshared var valid_count : uint;

interface IExposureInput
{
    func load(pixel: uint2, extent: uint2)->float4;
};

struct BufferInput : IExposureInput
{
    var values : StructuredBuffer<float4>;

    func load(pixel: uint2, extent: uint2)->float4
    {
        return values[pixel.y * extent.x + pixel.x];
    }
};

struct TextureInput : IExposureInput
{
    var values : Texture2D<float4>;

    func load(pixel: uint2, extent: uint2)->float4
    {
        return values.Load(int3(pixel, 0));
    }
};

func histogram_group<Input : IExposureInput>(input: Input, group: uint3, lane: uint3, index: uint)
    ->void
{
    var p : ExposureParameters = parameters[0];
    bins[index] = 0;
    if (index == 0)
        valid_count = 0;
    GroupMemoryBarrierWithGroupSync();
    let origin : uint2 = group.xy * 64 + lane.xy * 4;
    var count : uint = 0;
    for (var y: uint = 0; y < 4; ++y)
        for (var x: uint = 0; x < 4; ++x)
        {
            var pixel : uint2 = origin + uint2(x, y);
            if (any(pixel >= p.extent))
                continue;
            let value : float4 = input.load(pixel, p.extent);
            if (value.w < 0)
                continue;
            let bin : int = display::exposure_bin(value.xyz, p.linear709 != 0);
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

/// buffer 输入与 texture 输入使用同一测光内核；两种入口的输入均为 set0 binding1。
[shader("compute")]
[numthreads(16, 16, 1)]
func histogram_main([[vk::binding(1, 0)]] uniform linear_input: StructuredBuffer<float4>,
                    uint3 group: SV_GroupID, uint3 lane: SV_GroupThreadID,
                    uint index: SV_GroupIndex)
    ->void
{
    histogram_group(BufferInput(linear_input), group, lane, index);
}

[shader("compute")]
[numthreads(16, 16, 1)]
func histogram_texture_main([[vk::binding(1, 0)]] uniform linear_input: Texture2D<float4>,
                            uint3 group: SV_GroupID, uint3 lane: SV_GroupThreadID,
                            uint index: SV_GroupIndex)
    ->void
{
    histogram_group(TextureInput(linear_input), group, lane, index);
}

[shader("compute")]
[numthreads(1, 1, 1)]
func update_main()->void
{
    let p : ExposureParameters = parameters[0];
    var state : display::ExposureState = exposure_state[0];
    let valid : uint = histogram[256];
    let instant : bool = p.reset != 0 || p.instant != 0 || state.initialized == 0;
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
    let begin : uint = valid / 200;
    let end : uint = valid - begin;
    var cursor : uint = 0;
    var retained : uint = 0;
    var weighted_bins : float = 0;
    var minimum_bin : uint = 0;
    var maximum_bin : uint = 0;
    for (var bin: uint = 0; bin < 256; ++bin)
    {
        let next : uint = cursor + histogram[bin];
        let low : uint = max(cursor, begin);
        let high : uint = min(next, end);
        if (high > low)
        {
            if (retained == 0)
                minimum_bin = bin;
            maximum_bin = bin;
            weighted_bins += float(bin) * float(high - low);
            retained += high - low;
        }
        cursor = next;
    }
    if (retained == 0)
        return;
    state.measured_log_brightness = display::exposure_bin_log(weighted_bins / float(retained));
    let minimum = display::exposure_bin_log(minimum_bin);
    let maximum = display::exposure_bin_log(maximum_bin);
    state.target_ev = display::exposure_target_ev(state.measured_log_brightness, minimum, maximum,
                                                  p.compensation);
    state.exposure_ev =
        instant ? state.target_ev
                : display::exposure_adapt(state.exposure_ev, state.target_ev, p.delta_seconds);
    state.initialized = 1;
    exposure_state[0] = state;
}
