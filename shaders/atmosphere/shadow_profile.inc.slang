// Entry-local profile workspace. Original Prime/Intel profile algorithm.
// Copyright 2017 Intel Corporation; Apache-2.0. See THIRD_PARTY_NOTICES.md.
import atmosphere.shadow_math;
using namespace atmosphere;
public static const let SUN_SHADOW_CASCADE_COUNT = 5;
public static const let SUN_SHADOW_MAX_DISTANCE_METERS = 2048.0;
public static const let SUN_SHADOW_DEPTH_BIAS_METERS = 0.02;
public static const let SUN_SHADOW_CASCADE_RADIUS_TEXELS = 254.0;
public static const let SUN_SHADOW_TRAVERSAL_EPSILON = 1.0e-5;

public interface IShadowProfileSource {
    func texel_size(cascade: int)->float;
    func blocker_depth(cascade: int, world_node: int2)->float;
}

// Adapted from Intel's Outdoor Light Scattering Sample.
// Copyright 2017 Intel Corporation
// Licensed under the Apache License, Version 2.0. See
// LICENSES/intel-outdoor-light-scattering-Apache-2.0.txt and its NOTICE.
//
// Prime modification: the reference's raster-shadow line and 1D min/max texture
// are replaced by an exact leaf-cell profile over world-space RT clipmaps.
// Missing coverage and profile overflow stay shadowed to prevent cave light leaks.

public static const let SUN_SHADOW_PROFILE_NEAR_CAPACITY = 512;
public static const let SUN_SHADOW_PROFILE_FAR_CAPACITY = 256;
public static const let SUN_SHADOW_PROFILE_TOTAL_CAPACITY
    : int = SUN_SHADOW_PROFILE_NEAR_CAPACITY +
            (SUN_SHADOW_CASCADE_COUNT - 1) * SUN_SHADOW_PROFILE_FAR_CAPACITY;
public static const let SUN_SHADOW_PROFILE_EPSILON = 1.0e-4;

public groupshared var shadow_profile_basis_u : float3;
public groupshared var shadow_profile_basis_v : float3;
public groupshared var shadow_profile_direction_to_sun : float3;
public groupshared var shadow_profile_camera : float2;
public groupshared var shadow_profile_direction : float2;
public groupshared var shadow_profile_direction_max : float;
public groupshared var shadow_profile_camera_depth : float;
public groupshared var shadow_profile_counts : int[SUN_SHADOW_CASCADE_COUNT];
public groupshared var shadow_profile_entries : float2[SUN_SHADOW_PROFILE_TOTAL_CAPACITY];

public func initialize_shadow_profile(direction_to_sun: float3, projected_camera: float2,
                                      camera_depth: float, edge_direction: float3, basis_u: float3,
                                      basis_v: float3)
    ->void {
    shadow_profile_direction_to_sun = normalize(direction_to_sun);
    shadow_profile_basis_u = basis_u;
    shadow_profile_basis_v = basis_v;
    shadow_profile_camera = projected_camera;
    shadow_profile_camera_depth = camera_depth;
    let projected_edge = float2(dot(edge_direction, shadow_profile_basis_u),
                                dot(edge_direction, shadow_profile_basis_v));
    let projected_length = length(projected_edge);
    shadow_profile_direction =
        projected_length > 1.0e-6 ? projected_edge / projected_length : float2(1.0, 0.0);
    shadow_profile_direction_max =
        max(abs(shadow_profile_direction.x), abs(shadow_profile_direction.y));
}

func shadow_profile_base(cascade: int)->int {
    return cascade == 0
               ? 0
               : SUN_SHADOW_PROFILE_NEAR_CAPACITY + (cascade - 1) * SUN_SHADOW_PROFILE_FAR_CAPACITY;
}

func shadow_profile_capacity(cascade: int)->int {
    return cascade == 0 ? SUN_SHADOW_PROFILE_NEAR_CAPACITY : SUN_SHADOW_PROFILE_FAR_CAPACITY;
}

public func shadow_profile_cascade_end<Source>(source: Source, cascade: int)->float
    where Source : IShadowProfileSource {
    let radius : float = SUN_SHADOW_CASCADE_RADIUS_TEXELS * source.texel_size(cascade);
    return min(radius / max(shadow_profile_direction_max, SUN_SHADOW_PROFILE_EPSILON),
               SUN_SHADOW_MAX_DISTANCE_METERS);
}

public func build_shadow_profile_cascade<Source>(source: Source, cascade: int)->void
    where Source : IShadowProfileSource {
    let start_scalar = cascade == 0 ? 0.0 : shadow_profile_cascade_end(source, cascade - 1);
    let end_scalar = shadow_profile_cascade_end(source, cascade);
    if (end_scalar <= start_scalar + SUN_SHADOW_PROFILE_EPSILON) {
        shadow_profile_counts[cascade] = 0;
        return;
    }

    let texel_size : float = source.texel_size(cascade);
    let grid_rate = shadow_profile_direction / texel_size;
    // Only the toroidal storage coordinate matters. Keeping DDA coordinates
    // within one clipmap period prevents the directional boundary offset from
    // disappearing into the ULP of a large world-space grid coordinate.
    let absolute_grid_origin = shadow_profile_camera / texel_size;
    let grid_point =
        fmod(absolute_grid_origin + grid_rate * start_scalar, float(SUN_SHADOW_RESOLUTION));
    var cell = int2(floor(grid_point + sign(grid_rate) * SUN_SHADOW_PROFILE_EPSILON));
    let cell_step = int2(sign(grid_rate));
    var next_crossing = float2(1.0e20);
    var crossing_step = float2(1.0e20);
    if (grid_rate.x > 0.0) {
        next_crossing.x = (float(cell.x + 1) - grid_point.x) / grid_rate.x;
        crossing_step.x = 1.0 / grid_rate.x;
    } else if (grid_rate.x < 0.0) {
        next_crossing.x = (float(cell.x) - grid_point.x) / grid_rate.x;
        crossing_step.x = -1.0 / grid_rate.x;
    }
    if (grid_rate.y > 0.0) {
        next_crossing.y = (float(cell.y + 1) - grid_point.y) / grid_rate.y;
        crossing_step.y = 1.0 / grid_rate.y;
    } else if (grid_rate.y < 0.0) {
        next_crossing.y = (float(cell.y) - grid_point.y) / grid_rate.y;
        crossing_step.y = -1.0 / grid_rate.y;
    }
    let profile_length = end_scalar - start_scalar;
    var cursor = 0.0;
    var count = 0;
    let base = shadow_profile_base(cascade);
    let capacity = shadow_profile_capacity(cascade);

    for (; count < capacity; ++count) {
        let span_end : float = min(profile_length, min(next_crossing.x, next_crossing.y));
        if (span_end <= cursor + SUN_SHADOW_PROFILE_EPSILON) {
            shadow_profile_counts[cascade] = -1;
            return;
        }

        let resolved_shadow = source.blocker_depth(cascade, cell);
        shadow_profile_entries[base + count] = float2(start_scalar + span_end, resolved_shadow);
        cursor = span_end;
        if (cursor + SUN_SHADOW_PROFILE_EPSILON >= profile_length) {
            shadow_profile_counts[cascade] = count + 1;
            return;
        }

        let crosses_x : bool = next_crossing.x <= span_end + SUN_SHADOW_PROFILE_EPSILON;
        let crosses_y : bool = next_crossing.y <= span_end + SUN_SHADOW_PROFILE_EPSILON;
        if (!crosses_x && !crosses_y) {
            shadow_profile_counts[cascade] = -1;
            return;
        }
        if (crosses_x) {
            cell.x += cell_step.x;
            next_crossing.x += crossing_step.x;
        }
        if (crosses_y) {
            cell.y += cell_step.y;
            next_crossing.y += crossing_step.y;
        }
    }

    // A truncated profile is unknown, never lit.
    shadow_profile_counts[cascade] = -1;
}

func shadow_profile_find(cascade: int, scalar: float)->int {
    let count = shadow_profile_counts[cascade];
    let base = shadow_profile_base(cascade);
    var low = 0;
    var high : int = count;
    while (low < high) {
        let middle : int = (low + high) >> 1;
        if (shadow_profile_entries[base + middle].x <= scalar + SUN_SHADOW_PROFILE_EPSILON) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    return low;
}

func shadow_profile_cascade_lit_length(cascade: int, start_distance: float, end_distance: float,
                                       scalar_rate: float, inverse_scalar_rate: float,
                                       ray_depth_rate: float)
    ->float {
    let count = shadow_profile_counts[cascade];
    if (count <= 0 || end_distance <= start_distance) {
        return 0.0;
    }

    let base = shadow_profile_base(cascade);
    if (scalar_rate <= SUN_SHADOW_PROFILE_EPSILON) {
        return shadow_leaf_visible_length(
            shadow_profile_camera_depth, ray_depth_rate, start_distance, end_distance,
            shadow_profile_entries[base].y, SUN_SHADOW_DEPTH_BIAS_METERS);
    }

    let start_scalar = scalar_rate * start_distance;
    var index : int = shadow_profile_find(cascade, start_scalar);
    var cursor : float = start_distance;
    var lit_length = 0.0;
    let capacity = shadow_profile_capacity(cascade);
    for (var traversal = 0; traversal < capacity && index < count &&
                            cursor + SUN_SHADOW_TRAVERSAL_EPSILON < end_distance;
         ++traversal, ++index) {
        let entry = shadow_profile_entries[base + index];
        let span_end = min(entry.x * inverse_scalar_rate, end_distance);
        if (span_end <= cursor + SUN_SHADOW_TRAVERSAL_EPSILON) {
            continue;
        }
        lit_length +=
            shadow_leaf_visible_length(shadow_profile_camera_depth, ray_depth_rate, cursor,
                                       span_end, entry.y, SUN_SHADOW_DEPTH_BIAS_METERS);
        cursor = span_end;
    }
    // Any tail not represented by the profile is intentionally shadowed.
    return lit_length;
}

/// 同一相机射线在极线剖面中的固定速率；仅在正速率有效域内求倒数。
public struct SunShadowProfileRay {
    public var scalar_rate : float;
    public var inverse_scalar_rate : float;
    public var depth_rate : float;
    public var supported : bool;
};
/// profile初始化后准备同一射线的方向投影，供其各积分段复用。
public func shadow_profile_ray(ray_direction: float3)->SunShadowProfileRay {
    let projected_ray = float2(dot(ray_direction, shadow_profile_basis_u),
                               dot(ray_direction, shadow_profile_basis_v));
    let scalar_rate = dot(projected_ray, shadow_profile_direction);
    let perpendicular_rate = abs(projected_ray.x * shadow_profile_direction.y -
                                 projected_ray.y * shadow_profile_direction.x);
    let rate = max(scalar_rate, 0.0);
    return { rate, rate > SUN_SHADOW_PROFILE_EPSILON ? 1.0 / rate : 0.0,
             dot(ray_direction, shadow_profile_direction_to_sun),
             !(scalar_rate < -SUN_SHADOW_PROFILE_EPSILON || perpendicular_rate > 2.0e-4) };
}
/// 已准备射线的有限区间可见比例；保留缺失覆盖、剖面容量和区间阈值的原有语义。
public func shadow_visible_interval<Source>(source: Source, ray: SunShadowProfileRay,
                                            start_distance: float, end_distance: float)
    ->float
    where Source : IShadowProfileSource {
    if (end_distance <= start_distance) {
        return 1.0;
    }
    let traced_end = min(end_distance, SUN_SHADOW_MAX_DISTANCE_METERS);
    if (traced_end <= start_distance) {
        return 0.0;
    }

    if (!ray.supported)
        return 0.0;

    var cursor : float = start_distance;
    var lit_length = 0.0;
    for (var cascade = 0; cascade < SUN_SHADOW_CASCADE_COUNT && cursor < traced_end; ++cascade) {
        let cascade_end = ray.scalar_rate > SUN_SHADOW_PROFILE_EPSILON
                              ? min(traced_end, shadow_profile_cascade_end(source, cascade) *
                                                    ray.inverse_scalar_rate)
                              : traced_end;
        if (cascade_end > cursor) {
            lit_length +=
                shadow_profile_cascade_lit_length(cascade, cursor, cascade_end, ray.scalar_rate,
                                                  ray.inverse_scalar_rate, ray.depth_rate);
            cursor = cascade_end;
        }
    }
    return clamp(lit_length / (end_distance - start_distance), 0.0, 1.0);
}
/// 单区间调用；批量积分可先准备sunShadowProfileRay，避免逐段重复投影。
public func shadow_visible_interval<Source>(source: Source, ray_direction: float3,
                                            start_distance: float, end_distance: float)
    ->float
    where Source : IShadowProfileSource {
    return shadow_visible_interval(source, shadow_profile_ray(ray_direction), start_distance,
                                   end_distance);
}
