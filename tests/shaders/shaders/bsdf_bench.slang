/// Cornell Box 使用的 Lambertian 采样、立体角 PDF 和 BRDF。
module bsdf_bench;
import bsdf.lambert;
import sampling.pcg;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

[shader("compute")]
[numthreads(8, 8, 1)]
func lambert_main(id: uint3: SV_DispatchThreadID)->void {
    let pixel = id.y * 3840 + id.x;
    var state = pixel ^ seed[0].x;
    let slope = float2(id.xy) * (2.0 / float2(3840, 2160)) - 1.0;
    let normal = normalize(float3(slope, 1.0));
    let response = bsdf::lambertian_eval(float3(0.5) + 0.2 * normal);
    var result = float4(0.0);
    [loop]
    for (var draw = 0u; draw < 16; ++draw) {
        let u = sampling::random(state);
        let v = sampling::random(state);
        let sample = bsdf::lambertian_sample(normal, float2(u, v));
        result += float4(sample.direction * response, sample.pdf);
    }
    outputs[pixel] = result;
}
