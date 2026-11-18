/// 四边形灯的面积采样和几何连接，沿用实际立体角 PDF 与偏移端点。
module light_sampling_bench;
import geometry.ray;
import light_sampling.area;
import light_sampling.quad;
import "light_sampling/sample.slang";
import sampling.pcg;

[[vk::binding(0, 0)]]
var seed : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

[shader("compute")]
[numthreads(8, 8, 1)]
func emitter_main(id: uint3: SV_DispatchThreadID)->void {
    let pixel = id.y * 3840 + id.x;
    var state = pixel ^ seed[0].x;
    let receiver = float2(id.xy) / float2(3840, 2160) * 4.0 - 2.0;
    let point_value = geometry::SurfacePoint(float3(receiver, -2.0), float3(0, 0, 1), 1.0e-6);
    let width = 0.5 + sampling::random(state);
    let vertices = light_sampling::LightQuadVertices(float3(-1, -1, 0), float3(width, -1, 0),
                                                     float3(1, 1, 0.25), float3(-1, 1, 0));
    let proposal =
        light_sampling::LightSurfaceProposal(light_sampling::dynamic_quad_area(vertices), 0);
    var sum = float4(0.0);
    [loop]
    for (var draw = 0u; draw < 8; ++draw) {
        let u = sampling::random(state);
        let v = sampling::random(state);
        var area : light_sampling::QuadAreaSample;
        let sample = light_sampling::sample_quad_emitter(light_sampling::UniformEmitterAlias(),
                                                         proposal, vertices, point_value,
                                                         float2(u, v), 1.0 / 3000000.0, 3u, area);
        sum += float4(sample.visibility.direction, sample.pdf);
    }
    outputs[pixel] = sum;
}
