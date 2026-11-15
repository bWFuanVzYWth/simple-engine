// Entry-local resource binding. Five original 512-square toroidal leaf grids.
// Request epoch, world X/Y and blocker depth per cell.

func shadowIndex(cascade: int, cell: int2)->uint
{
    let wrapped : uint2 = uint2(cell) & 511u;
    return uint(cascade) * 512u * 512u + wrapped.y * 512u + wrapped.x;
}
struct ShadowSource<let Demand : bool> : IPrimeSunShadowProfileSource
{
    func texelSize(cascade: int)->float
    {
        return sunShadowTexelSize(cascade);
    }
    func blockerDepth(cascade: int, worldNode: int2)->float
    {
        var depth : float = -1e20;
        for (var y: int = -1; y <= 1; y++)
            for (var x: int = -1; x <= 1; x++)
            {
                let index : uint = shadowIndex(cascade, worldNode + int2(x, y));
                if (Demand)
                {
                    var previous : uint;
                    InterlockedOr(shadowCells[index].x, 0x80000000u, previous);
                }
                else
                {
                    let cell : uint4 = shadowCells[index];
                    let f : TerrainFrame = frames[0];
                    let tag : uint2 = terrain_column(f, cascade, worldNode + int2(x, y));
                    var h : float = asfloat(cell.w);
                    if (h > -1e19)
                    {
                        h -= f.source.basisV.w;
                        h -= floor((h + 16384.0) / 32768.0) * 32768.0;
                    }
                    depth = max(depth, all(cell.yz == tag) && cell.w != 0 ? h : 1e20);
                }
            }
        return depth;
    }
};
func shadowSource<let Demand : bool>()->ShadowSource<Demand>
{
    return {};
}

func terrain_slice<let Demand : bool>(lane: uint, slice: uint)->void
{
    if (lane == 0)
    {
        let f : TerrainFrame = frames[0];
        primeInitializeSunShadowProfile(f.source.sun.xyz, f.source.grid.xy, -f.source.basisU.w,
                                        terrain_ray(f, 127, slice), f.source.basisU.xyz,
                                        f.source.basisV.xyz);
    }
    GroupMemoryBarrierWithGroupSync();
    if (lane < SUN_SHADOW_CASCADE_COUNT)
        sunShadowBuildProfileCascade(shadowSource<Demand>(), int(lane));
    GroupMemoryBarrierWithGroupSync();
}
