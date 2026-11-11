module terrain_frame;
import atmosphere.epipolar;
import atmosphere.shadow_math;
namespace atmosphere
{
public static const uint TERRAIN_RADIAL = 128;
public static const uint TERRAIN_SLICES = 256;
public static const uint TERRAIN_DEPTH = 128;
public static const float TERRAIN_RANGE_M = 2048.0;
// TLAS 局部坐标以米计，高度以 km 计。输入按一次场景快照发布。
public struct TerrainInput
{
    public float4 camera;
    public float4 forward;
    public float4 right;
    public float4 up;
    public float4 sun;
    public float4 scale;
    public uint4 state;
    public float4 basisU;
    public float4 basisV;
    public float4 grid;
};
public struct TerrainFrame
{
    public TerrainInput source;
    public float4 projection;
    public float4 angular;
};
public TerrainFrame terrain_frame(TerrainInput input)
{
    TerrainFrame f;
    f.source = input;
    float3 forward = input.forward.xyz, right = input.right.xyz, up = input.up.xyz;
    float3 sun = input.sun.xyz;
    float3 coordinates = float3(dot(sun, cross(up, forward)), dot(sun, cross(forward, right)),
                                dot(sun, cross(right, up))) /
                         dot(forward, cross(right, up));
    float2 projected = abs(coordinates.z) > 1e-6
                           ? coordinates.xy / coordinates.z
                           : (dot(coordinates.xy, coordinates.xy) > 1e-12
                                  ? normalize(coordinates.xy) * ATM_EPIPOLAR_LIMIT
                                  : float2(0));
    projected = clamp(projected, -ATM_EPIPOLAR_LIMIT, ATM_EPIPOLAR_LIMIT);
    AtmEpipolarRange range = atmEpipolarVisibleRange(projected);
    f.projection = float4(projected, range.start, range.span);
    f.angular = float4(range.axis, range.extent, float(range.parallel));
    return f;
}
public AtmEpipolarRange terrain_range(TerrainFrame f)
{
    return { f.projection.z, f.projection.w, f.angular.xy, f.angular.z, f.angular.w != 0 };
}
public float3 terrain_ray(TerrainFrame f, uint lane, uint slice)
{
    AtmEpipolarSegment segment =
        atmEpipolarSliceSegment(f.projection.xy, terrain_range(f), int(slice), int(TERRAIN_SLICES));
    float2 ndc = lerp(segment.entry, segment.exit, (float(lane) + 0.5) / float(TERRAIN_RADIAL));
    return normalize(f.source.forward.xyz + ndc.x * f.source.right.xyz + ndc.y * f.source.up.xyz);
}
// 冷启动覆盖全部切片。自然刷新每帧更新八分之一，按实际提交轮转。
public bool terrain_full(TerrainFrame f)
{
    return (f.source.state.w & 8u) != 0;
}
public uint terrain_slice_index(TerrainFrame f, uint group)
{
    return terrain_full(f) ? group : group * 8u + (f.source.state.w & 7u);
}
public uint2 terrain_column(TerrainFrame f, int cascade, int2 wrapped)
{
    float size = sunShadowTexelSize(cascade);
    int2 lo = int2(floor(f.source.grid.xy / size)) - 256;
    return asuint(f.source.grid.zw) * (8192u >> uint(cascade)) +
           asuint(lo + ((wrapped - lo) & 511));
}
}
