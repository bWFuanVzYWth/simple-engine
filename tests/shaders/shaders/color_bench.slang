// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

module color_bench;
import color.hsv;
import color.primaries;
import color.srgb;
import display.prime_drt;

// 前两项是 Prime DRT 参数，其后是按行存储的 3840×2160 RGB 像素。
[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

static const let EXPOSURE_COUNT = 64u;

func pixel_index(id: uint3)->uint {
    return id.y * 3840 + id.x;
}

func exposure_gain(index: uint)->float {
    return 1.0 + (1.0 / 32.0) * float(index);
}

[shader("compute")]
[numthreads(16, 16, 1)]
func primaries_main(id: uint3: SV_DispatchThreadID)->void {
    let i = pixel_index(id);
    let rgb = inputs[i + 2].xyz;
    var sum = float4(0.0);
    [loop]
    for (var exposure = 0u; exposure < EXPOSURE_COUNT; ++exposure) {
        let rec2020 = color::bt709_to_rec2020(rgb * exposure_gain(exposure));
        sum += float4(color::rec2020_to_bt709(rec2020), color::rec2020_luminance(rec2020));
    }
    outputs[i] = sum;
}

[shader("compute")]
[numthreads(16, 16, 1)]
func srgb_main(id: uint3: SV_DispatchThreadID)->void {
    let i = pixel_index(id);
    let rgb = inputs[i + 2].xyz;
    var sum = float4(0.0);
    [loop]
    for (var exposure = 0u; exposure < EXPOSURE_COUNT; ++exposure) {
        let srgb = color::encode_srgb(rgb * exposure_gain(exposure));
        sum += float4(color::decode_srgb(srgb), srgb.x);
    }
    outputs[i] = sum;
}

[shader("compute")]
[numthreads(16, 16, 1)]
func hsv_main(id: uint3: SV_DispatchThreadID)->void {
    let i = pixel_index(id);
    let rgb = inputs[i + 2].xyz;
    var sum = float4(0.0);
    [loop]
    for (var exposure = 0u; exposure < EXPOSURE_COUNT; ++exposure) {
        let hsv = color::rgb_to_hsv(rgb * exposure_gain(exposure));
        sum += float4(color::hsv_to_rgb(hsv), hsv.x);
    }
    outputs[i] = sum;
}

[shader("compute")]
[numthreads(16, 16, 1)]
func prime_drt_main(id: uint3: SV_DispatchThreadID)->void {
    let i = pixel_index(id);
    let a = inputs[0];
    let b = inputs[1];
    let parameters : display::PrimeDrtParameters = { a.x, a.y, a.z, a.w, b.x, b.y, b.zw };
    outputs[i] = float4(display::prime_drt_rec2020(inputs[i + 2].xyz, parameters), 1.0);
}
