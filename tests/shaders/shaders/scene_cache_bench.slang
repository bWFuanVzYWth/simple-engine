/// 读取实际 Rust 目录编码，混合已发布和不存在的 token。
module scene_cache_bench;
import scene_cache.identity;
import sampling.pcg;

[[vk::binding(0, 0)]]
var words : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

[shader("compute")]
[numthreads(8, 8, 1)]
func identity_main(id: uint3: SV_DispatchThreadID)->void {
    let pixel = id.y * 3840 + id.x;
    var state = pixel ^ words[0].x;
    var sum = 0u;
    var hits = 0u;
    var valid = true;
    [loop]
    for (var draw = 0u; draw < 8; ++draw) {
        state = sampling::pcg(state);
        let token = state & 0x1ffffu;
        let exists = (token & 1u) != 0;
        let index = scene_cache::identity_index(words, 1, token);
        valid &= index == select(exists, token >> 1, 0xffffffffu);
        if (index != 0xffffffffu) {
            sum += index;
            ++hits;
        }
    }
    outputs[pixel] = float4(float(sum), float(hits), 0.0, select(valid, 1.0, asfloat(0x7fc00000u)));
}
