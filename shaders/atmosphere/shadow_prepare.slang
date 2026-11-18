import atmosphere.terrain_frame;
using namespace atmosphere;
[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<TerrainInput>;
[[vk::binding(1, 0)]]
var frames : RWStructuredBuffer<TerrainFrame>;
[shader("compute")]
[numthreads(1, 1, 1)]
func shadow_prepare()->void {
    frames[0] = terrain_frame(inputs[0]);
}
