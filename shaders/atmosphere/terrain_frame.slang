module terrain_frame;
import atmosphere.epipolar;
import atmosphere.shadow_math;
namespace atmosphere
{
public static const let TERRAIN_RADIAL : uint = 128;
public static const let TERRAIN_SLICES : uint = 256;
public static const let TERRAIN_DEPTH : uint = 128;
public static const let TERRAIN_RANGE_M = 2048.0;
// TLAS 局部坐标以米计，高度以 km 计。输入按一次场景快照发布。
public struct TerrainInput
{
    public var camera : float4;
    public var forward : float4;
    public var right : float4;
    public var up : float4;
    public var sun : float4;
    public var scale : float4;
    public var state : uint4;
    public var basisU : float4;
    public var basisV : float4;
    public var grid : float4;
};
public struct TerrainFrame
{
    public var source : TerrainInput;
    public var projection : float4;
    public var angular : float4;
};
public func terrain_frame(input: TerrainInput)->TerrainFrame
{
    var f : TerrainFrame;
    f.source = input;
    let forward : float3 = input.forward.xyz;
    let right : float3 = input.right.xyz;
    let up : float3 = input.up.xyz;
    let sun : float3 = input.sun.xyz;
    let coordinates = float3(dot(sun, cross(up, forward)), dot(sun, cross(forward, right)),
                             dot(sun, cross(right, up))) /
                      dot(forward, cross(right, up));
    var projected : float2 = abs(coordinates.z) > 1e-6
                                 ? coordinates.xy / coordinates.z
                                 : (dot(coordinates.xy, coordinates.xy) > 1e-12
                                        ? normalize(coordinates.xy) * ATM_EPIPOLAR_LIMIT
                                        : float2(0));
    projected = clamp(projected, -ATM_EPIPOLAR_LIMIT, ATM_EPIPOLAR_LIMIT);
    let range : AtmEpipolarRange = atmEpipolarVisibleRange(projected);
    f.projection = float4(projected, range.start, range.span);
    f.angular = float4(range.axis, range.extent, float(range.parallel));
    return f;
}
public func terrain_range(f: TerrainFrame)->AtmEpipolarRange
{
    return { f.projection.z, f.projection.w, f.angular.xy, f.angular.z, f.angular.w != 0 };
}
public func terrain_ray(f: TerrainFrame, lane: uint, slice: uint)->float3
{
    let segment : AtmEpipolarSegment = atmEpipolarSliceSegment(f.projection.xy, terrain_range(f),
                                                               int(slice), int(TERRAIN_SLICES));
    let ndc : float2 =
                  lerp(segment.entry, segment.exit, (float(lane) + 0.5) / float(TERRAIN_RADIAL));
    return normalize(f.source.forward.xyz + ndc.x * f.source.right.xyz + ndc.y * f.source.up.xyz);
}
// 冷启动覆盖全部切片。自然刷新每帧更新八分之一，按实际提交轮转。
public func terrain_full(f: TerrainFrame)->bool
{
    return (f.source.state.w & 8u) != 0;
}
public func terrain_slice_index(f: TerrainFrame, group: uint)->uint
{
    return terrain_full(f) ? group : group * 8u + (f.source.state.w & 7u);
}
public func terrain_column(f: TerrainFrame, cascade: int, wrapped: int2)->uint2
{
    let size : float = sunShadowTexelSize(cascade);
    let lo : int2 = int2(floor(f.source.grid.xy / size)) - 256;
    return asuint(f.source.grid.zw) * (8192u >> uint(cascade)) +
           asuint(lo + ((wrapped - lo) & 511));
}
}
