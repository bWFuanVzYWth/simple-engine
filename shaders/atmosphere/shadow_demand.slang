import atmosphere.terrain_frame;
using namespace atmosphere;
#include "shadow_profile.inc.slang"
[[vk::binding(0, 0)]]
StructuredBuffer<TerrainFrame> frames;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> shadowCells;
#define ATM_SHADOW_DEMAND
#include "shadow_source.inc.slang"
[shader("compute")]
[numthreads(8, 1, 1)]
void shadow_demand(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
{
    terrain_slice(lane, terrain_slice_index(frames[0], group.y));
}
