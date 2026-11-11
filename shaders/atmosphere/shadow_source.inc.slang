// Entry-local resource binding. Five original 512-square toroidal leaf grids.
// Request epoch, world X/Y and blocker depth per cell.

uint shadowIndex(int cascade, int2 cell)
{
    uint2 wrapped = uint2(cell) & 511u;
    return uint(cascade) * 512u * 512u + wrapped.y * 512u + wrapped.x;
}
struct ShadowSource : IPrimeSunShadowProfileSource
{
    float texelSize(int cascade)
    {
        return sunShadowTexelSize(cascade);
    }
    float blockerDepth(int cascade, int2 worldNode)
    {
        float depth = -1e20;
        for (int y = -1; y <= 1; y++)
            for (int x = -1; x <= 1; x++)
            {
                uint index = shadowIndex(cascade, worldNode + int2(x, y));
#ifdef ATM_SHADOW_DEMAND
                uint previous;
                InterlockedOr(shadowCells[index].x, 0x80000000u, previous);
#else
                uint4 cell = shadowCells[index];
                TerrainFrame f = frames[0];
                uint2 tag = terrain_column(f, cascade, worldNode + int2(x, y));
                float h = asfloat(cell.w);
                if (h > -1e19)
                {
                    h -= f.source.basisV.w;
                    h -= floor((h + 16384.0) / 32768.0) * 32768.0;
                }
                depth = max(depth, all(cell.yz == tag) && cell.w != 0 ? h : 1e20);
#endif
            }
        return depth;
    }
};
ShadowSource shadowSource()
{
    return {};
}

void terrain_slice(uint lane, uint slice)
{
    if (lane == 0)
    {
        TerrainFrame f = frames[0];
        primeInitializeSunShadowProfile(f.source.sun.xyz, f.source.grid.xy, -f.source.basisU.w,
                                        terrain_ray(f, 127, slice), f.source.basisU.xyz,
                                        f.source.basisV.xyz);
    }
    GroupMemoryBarrierWithGroupSync();
    if (lane < SUN_SHADOW_CASCADE_COUNT)
        sunShadowBuildProfileCascade(shadowSource(), int(lane));
    GroupMemoryBarrierWithGroupSync();
}
