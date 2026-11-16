// Entry-local resource binding. Five original 512-square toroidal leaf grids.
// Request epoch, world X/Y and blocker depth per cell.
groupshared var terrain_segment_endpoints : float4;

func shadow_index(cascade: int, cell: int2)->uint
{
    let wrapped = uint2(cell) & 511u;
    return uint(cascade) * 512u * 512u + wrapped.y * 512u + wrapped.x;
}
struct ShadowSource<let demand : bool> : IShadowProfileSource
{
    func texel_size(cascade: int)->float
    {
        return shadow_texel_size(cascade);
    }
    func blocker_depth(cascade: int, world_node: int2)->float
    {
        var depth : float = -1e20;
        for (var y: int = -1; y <= 1; y++)
            for (var x: int = -1; x <= 1; x++)
            {
                let index = shadow_index(cascade, world_node + int2(x, y));
                if (demand)
                {
                    var previous : uint;
                    InterlockedOr(shadow_cells[index].x, 0x80000000u, previous);
                }
                else
                {
                    let cell = shadow_cells[index];
                    let f : TerrainFrame = frames[0];
                    let tag = terrain_column(f, cascade, world_node + int2(x, y));
                    var h : float = asfloat(cell.w);
                    if (h > -1e19)
                    {
                        h -= f.source.wrapped_shadow_plane_depth;
                        h -= floor((h + 16384.0) * (1.0 / 32768.0)) * 32768.0;
                    }
                    depth = max(depth, all(cell.yz == tag) && cell.w != 0 ? h : 1e20);
                }
            }
        return depth;
    }
};
func shadow_source<let demand : bool>()->ShadowSource<demand>
{
    return {};
}

func terrain_slice<let demand : bool>(lane: uint, slice: uint)->void
{
    if (lane == 0)
    {
        let f : TerrainFrame = frames[0];
        let segment = epipolar_slice_segment(f.projection.xy, terrain_range(f), int(slice),
                                             int(TERRAIN_SLICES));
        if (!demand)
            terrain_segment_endpoints = float4(segment.entry, segment.exit);
        initialize_shadow_profile(f.source.sun_direction, f.source.wrapped_grid,
                                  -f.source.sun_depth_offset, terrain_ray(f, 127, segment),
                                  f.source.basis_u, f.source.basis_v);
    }
    GroupMemoryBarrierWithGroupSync();
    if (lane < SUN_SHADOW_CASCADE_COUNT)
        build_shadow_profile_cascade(shadow_source<demand>(), int(lane));
    GroupMemoryBarrierWithGroupSync();
}
