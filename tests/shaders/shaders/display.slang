module display_test;

import display.prime_drt;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

func parameters(a: float4, b: float4)->display::PrimeDrtParameters {
    return { a.x, a.y, a.z, a.w, b.x, b.y, b.zw };
}

[shader("compute")]
[numthreads(64, 1, 1)]
func display_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    let i : uint = id.x;
    if (i >= rows / 4)
        return;
    let color : float3 = asfloat(inputs[4 * i]).xyz;
    let p : display::PrimeDrtParameters =
                parameters(asfloat(inputs[4 * i + 1]), asfloat(inputs[4 * i + 2]));
    let mode : uint = inputs[4 * i + 3].y;
    let result : float3 = mode == 1   ? display::prime_drt_rec2020(color, p)
                          : mode == 2 ? display::prime_drt_linear_bt709(color, p)
                                      : display::prime_drt_bt709(color, p);
    outputs[i] = uint4(asuint(result), i + 1);
}
