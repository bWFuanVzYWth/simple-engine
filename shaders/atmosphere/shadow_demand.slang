import atmosphere.terrain_frame;
using namespace atmosphere;
#include "shadow_profile.inc.slang"
[[vk::binding(0, 0)]]
var frames : StructuredBuffer<TerrainFrame>;
[[vk::binding(1, 0)]]
var shadow_cells : RWStructuredBuffer<uint4>;
#include "shadow_source.inc.slang"
[shader("compute")]
[numthreads(8, 1, 1)]
func shadow_demand(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void {
    terrain_slice<true>(lane, terrain_slice_index(frames[0], group.y));
}
