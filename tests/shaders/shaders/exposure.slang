/// 曝光生产状态的回读适配；不重新实现计量或更新算法。
module exposure_readback;
import display.exposure;

[[vk::binding(0, 0)]]
var exposure_state : StructuredBuffer<display::ExposureState>;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<uint4>;

[shader("compute")]
[numthreads(1, 1, 1)]
func readback_main()->void
{
    let state : display::ExposureState = exposure_state[0];
    result[0] = uint4(asuint(state.exposure_ev), state.initialized, asuint(state.target_ev),
                      asuint(state.measured_log_brightness));
}
