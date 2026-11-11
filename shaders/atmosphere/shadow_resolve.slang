module shadow_resolve;
import atmosphere.terrain_frame;
import atmosphere.shadow_math;
namespace atmosphere
{
/// 应用只判定候选三角形的实际覆盖；硬件 OMM 的确定态由遍历处理。
public interface IBlockerCoverage
{
    static bool accepts(uint instance, uint primitive, float2 bary, bool front);
}
// 相机射线遵循主查询的覆盖朝向。阻挡列从太阳侧向内查询，覆盖须取接收点向太阳看的那一面。
float terrain_hit_distance<C : IBlockerCoverage, let reverse : bool>(
    RaytracingAccelerationStructure scene, RayDesc ray)
{
    RayQuery<RAY_FLAG_NONE> q;
    q.TraceRayInline(scene, RAY_FLAG_NONE, 0xff, ray);
    while (q.Proceed())
    {
        if (q.CandidateType() == CANDIDATE_NON_OPAQUE_TRIANGLE &&
            C.accepts(q.CandidateInstanceID(), q.CandidatePrimitiveIndex(),
                      q.CandidateTriangleBarycentrics(), q.CandidateTriangleFrontFace() != reverse))
            q.CommitNonOpaqueTriangleHit();
    }
    return q.CommittedStatus() == COMMITTED_TRIANGLE_HIT ? q.CommittedRayT() : ray.TMax;
}

public void resolve_shadow_column<C : IBlockerCoverage>(uint3 id, TerrainFrame f,
                                                        RWStructuredBuffer<uint4> cells,
                                                        RaytracingAccelerationStructure scene,
                                                        RWStructuredBuffer<uint> first_hits)
{
    // 与当前刷新批共用 TLAS 和覆盖规则。最高位通知积分阶段本射线的终点是否实际改变。
    uint slices = terrain_full(f) ? TERRAIN_SLICES : TERRAIN_SLICES / 8u;
    if (id.z == 0 && id.x < TERRAIN_RADIAL && id.y < slices)
    {
        uint slice = terrain_slice_index(f, id.y), sample = slice * TERRAIN_RADIAL + id.x;
        RayDesc camera = { f.source.camera.xyz, 0, terrain_ray(f, id.x, slice), TERRAIN_RANGE_M };
        uint bits = asuint(terrain_hit_distance<C, false>(scene, camera));
        bool changed = terrain_full(f) || (first_hits[sample] & 0x7fffffffu) != bits;
        first_hits[sample] = bits | (uint(changed) << 31u);
    }
    uint index = (id.z * 512u + id.y) * 512u + id.x;
    uint4 cell = cells[index];
    if ((cell.x & 0x80000000u) == 0)
        return;
    uint last = cell.x & 0x7fffffffu;
    float size = sunShadowTexelSize(int(id.z));
    int2 lo = int2(floor(f.source.grid.xy / size)) - 256;
    int2 localCell = lo + ((int2(id.xy) - lo) & 511);
    uint2 tag = terrain_column(f, int(id.z), int2(id.xy));
    // 重叠切片在四个已提交活跃帧内复用同一列。局部场景发布不清理相邻格。
    if (all(cell.yz == tag) && cell.w != 0 && ((f.source.state.x - last) & 0x7fffffffu) < 4u)
    {
        cells[index].x = last;
        return;
    }
    float2 delta = (float2(localCell) + 0.5) * size - f.source.grid.xy;
    float3 origin = f.source.camera.xyz + f.source.sun.xyz * (f.source.basisU.w + 4096.0) +
                    f.source.basisU.xyz * delta.x + f.source.basisV.xyz * delta.y;
    RayDesc ray = { origin, 0, -f.source.sun.xyz, 8192.0 };
    float distance = terrain_hit_distance<C, true>(scene, ray);
    float blocker = -1e20;
    if (distance < ray.TMax)
    {
        blocker = f.source.basisV.w + 4096.0 - distance;
        blocker -= floor(blocker / 32768.0) * 32768.0;
    }
    cells[index] = uint4(f.source.state.x, tag, blocker == 0 ? 0x80000000u : asuint(blocker));
}
}
