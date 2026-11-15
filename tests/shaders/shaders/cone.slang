module cone_test;
import geometry.cone;
[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;
[shader("compute")]
[numthreads(64, 1, 1)]
func cone_main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    inputs.GetDimensions(count, stride);
    if (2 * id.x >= count)
        return;
    let cone : float4 = inputs[2 * id.x];
    let chord = geometry::cone_chord_squared(cone.w);
    let d = geometry::sample_uniform_cone_chord(cone.xyz, chord, inputs[2 * id.x + 1].xy);
    outputs[2 * id.x] = float4(d, geometry::uniform_cone_pdf_chord(chord));
    outputs[2 * id.x + 1] = float4(geometry::cone_contains_chord(cone.xyz, chord, d), 0, 0, 0);
}
