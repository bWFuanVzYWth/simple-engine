module light_geometry;
import pt.light_sampling;
import geometry.ray;

[[vk::binding(0, 0)]]
StructuredBuffer<float4> input;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> output;

[shader("compute")]
[numthreads(64, 1, 1)]
func light_geometry_main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    input.GetDimensions(count, stride);
    if (id.x >= count / 3u)
        return;
    let receiver = input[3 * id.x];
    let target = input[3 * id.x + 1];
    let normal = input[3 * id.x + 2];
    let delta = receiver.xyz - target.xyz;
    let squared = dot(delta, delta);
    let cosine = squared > 0 && isfinite(squared) ? dot(normal.xyz, delta * rsqrt(squared)) : 0;
    output[id.x] = float4(
        pt::quad_light_pdf(receiver.xyz, target.xyz, normal.xyz, receiver.w, target.w,
                           asuint(normal.w)),
        pt::quad_light_pdf_geometry(squared, cosine, receiver.w, target.w, asuint(normal.w)), 0, 0);
}

[shader("compute")]
[numthreads(64, 1, 1)]
func uniform_area_main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    input.GetDimensions(count, stride);
    if (id.x >= (count >> 1u))
        return;
    let params = input[2 * id.x];
    let uv = input[2 * id.x + 1];
    let area = asuint(params);
    let sample = pt::UniformEmitterAlias().sample( { area, 0 }, uv.xy);
    output[id.x] = float4(asfloat(sample.half), sample.bary, sample.density);
}

[shader("compute")]
[numthreads(64, 1, 1)]
func emitter_connection_main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    input.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let p = input[id.x];
    let source : geometry::SurfacePoint = { float3(0, 0, p.x), float3(0, 0, -1), 0 };
    let vertices : pt::LightQuadVertices = { float3(-1, -1, 0), float3(1, -1, 0), float3(1, 1, 0),
                                             float3(-1, 1, 0) };
    let proposal
        : pt::LightSurfaceProposal = { uint4(1u << 23u, asuint(0.25), asuint(0.25), 0), 0 };
    var area : pt::QuadAreaSample;
    let sample = pt::sample_quad_emitter(pt::UniformEmitterAlias(), proposal, vertices, source,
                                         p.yz, 0.4, pt::LIGHT_FRONT, area);
    let target = pt::quad_endpoint(vertices, area);
    output[id.x] = float4(sample.distance, sample.pdf, target.position.xy);
}
