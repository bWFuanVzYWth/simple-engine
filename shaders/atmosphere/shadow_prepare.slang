import atmosphere.terrain_frame;
using namespace atmosphere;
[[vk::binding(0, 0)]]
StructuredBuffer<TerrainInput> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<TerrainFrame> frames;
[shader("compute")]
[numthreads(1, 1, 1)]
void shadow_prepare()
{
    frames[0] = terrain_frame(inputs[0]);
}
