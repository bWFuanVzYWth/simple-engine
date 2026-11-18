module terrain_frame;
import atmosphere.epipolar;
import atmosphere.shadow_math;
namespace atmosphere {
public static const let TERRAIN_RADIAL : uint = 128;
public static const let TERRAIN_SLICES : uint = 256;
public static const let TERRAIN_DEPTH : uint = 128;
public static const let TERRAIN_RANGE_M = 2048.0;
// TLAS 局部坐标以米计，高度以 km 计。输入按一次场景快照发布。
public struct TerrainInput {
    public var camera_position : float3;
    public var observer_height_km : float;
    public var forward : float3;
    public var forward_padding : float;
    public var right : float3;
    public var right_padding : float;
    public var up : float3;
    public var up_padding : float;
    public var sun_direction : float3;
    public var sun_padding : float;
    public var radiance_scale : float3;
    public var scale_padding : float;
    public var refresh_clock : uint;
    public var active : uint;
    public var view_version : uint;
    public var refresh_bits : uint;
    public var basis_u : float3;
    public var sun_depth_offset : float;
    public var basis_v : float3;
    public var wrapped_shadow_plane_depth : float;
    public var wrapped_grid : float2;
    public var grid_tile : uint2;
};
public struct TerrainFrame {
    public var source : TerrainInput;
    public var projection : float4;
    public var angular : float4;
};
public func terrain_frame(input: TerrainInput)->TerrainFrame {
    var f : TerrainFrame;
    f.source = input;
    let forward = input.forward;
    let right = input.right;
    let up = input.up;
    let sun = input.sun_direction;
    let coordinates = float3(dot(sun, cross(up, forward)), dot(sun, cross(forward, right)),
                             dot(sun, cross(right, up))) /
                      dot(forward, cross(right, up));
    var projected : float2 = abs(coordinates.z) > 1e-6
                                 ? coordinates.xy / coordinates.z
                                 : (dot(coordinates.xy, coordinates.xy) > 1e-12
                                        ? normalize(coordinates.xy) * EPIPOLAR_LIMIT
                                        : float2(0));
    projected = clamp(projected, -EPIPOLAR_LIMIT, EPIPOLAR_LIMIT);
    let range = epipolar_visible_range(projected);
    f.projection = float4(projected, range.start, range.span);
    f.angular = float4(range.axis, range.extent, float(range.parallel));
    return f;
}
public func terrain_range(f: TerrainFrame)->EpipolarRange {
    return { f.projection.z, f.projection.w, f.angular.xy, f.angular.z, f.angular.w != 0 };
}
/// 在同一极线段内按径向lane重建单位相机射线；段可由工作组共享。
public func terrain_ray(f: TerrainFrame, lane: uint, segment: EpipolarSegment)->float3 {
    let ndc : float2 =
                  lerp(segment.entry, segment.exit, (float(lane) + 0.5) / float(TERRAIN_RADIAL));
    return normalize(f.source.forward + ndc.x * f.source.right + ndc.y * f.source.up);
}
/// 独立查询的相机射线；批量同切片调用可先求极线段。
public func terrain_ray(f: TerrainFrame, lane: uint, slice: uint)->float3 {
    let segment =
        epipolar_slice_segment(f.projection.xy, terrain_range(f), int(slice), int(TERRAIN_SLICES));
    return terrain_ray(f, lane, segment);
}
// 冷启动覆盖全部切片。自然刷新每帧更新八分之一，按实际提交轮转。
public func terrain_full(f: TerrainFrame)->bool {
    return (f.source.refresh_bits & 8u) != 0;
}
public func terrain_slice_index(f: TerrainFrame, group: uint)->uint {
    return terrain_full(f) ? group : group * 8u + (f.source.refresh_bits & 7u);
}
public func terrain_column(f: TerrainFrame, cascade: int, wrapped: int2)->uint2 {
    let size = shadow_texel_size(cascade);
    let lo = int2(floor(f.source.wrapped_grid / size)) - 256;
    return f.source.grid_tile * (8192u >> uint(cascade)) + asuint(lo + ((wrapped - lo) & 511));
}
}
