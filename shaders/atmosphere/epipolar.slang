// 极线参数化与相邻切片重建移植自 Prime PT e0260fc4。
module epipolar;
import math.constants;
namespace atmosphere {

public static const let EPIPOLAR_LIMIT = 16384.0;
public static const let EPIPOLAR_EPSILON = 1.0e-6;

public struct EpipolarSegment {
    public var entry : float2;
    public var exit : float2;
    public var valid : bool;
};

public struct EpipolarRange {
    public var start : float;
    public var span : float;
    public var axis : float2;
    public var inverse_distance : float;
    public var transverse : bool;
};

public func sort_epipolar_pair(inout first: float, inout second: float)->void {
    if (first > second) {
        let temporary = first;
        first = second;
        second = temporary;
    }
}

// 侧向太阳用极线在垂直于 axis 的中心轴上的截距编号。齐次深度为零时自然得到平行线。
func intercept(axis: float2, inverse_distance: float, p: float2)->float {
    return dot(float2(-axis.y, axis.x), p) / (1. + inverse_distance * dot(axis, p));
}
public func epipolar_visible_range(projected_sun: float3, out epipole_ndc: float2)->EpipolarRange {
    var result : EpipolarRange;
    result.axis = float2(0.0);
    result.inverse_distance = 0.;
    // 太阳投影超过八个屏幕半径时用截距，四角的分母均远离零。
    result.transverse = max(abs(projected_sun.x), abs(projected_sun.y)) > 8. * abs(projected_sun.z);
    if (result.transverse) {
        epipole_ndc = 0.;
        let inverse_length = rsqrt(dot(projected_sun.xy, projected_sun.xy));
        result.axis = -projected_sun.xy * inverse_length;
        result.inverse_distance = projected_sun.z * inverse_length;
        let a = result.axis;
        let q = result.inverse_distance;
        let bounds = float4(intercept(a, q, float2(-1, -1)), intercept(a, q, float2(1, -1)),
                            intercept(a, q, float2(-1, 1)), intercept(a, q, float2(1, 1)));
        result.start = min(min(bounds.x, bounds.y), min(bounds.z, bounds.w));
        result.span = max(max(bounds.x, bounds.y), max(bounds.z, bounds.w)) - result.start;
        return result;
    }
    epipole_ndc = projected_sun.xy / projected_sun.z;
    if (all(abs(epipole_ndc) <= float2(1.0))) {
        result.start = -math::pi;
        result.span = math::tau;
        return result;
    }

    var angle0 : float =
                     fmod(atan2(-1.0 - epipole_ndc.y, -1.0 - epipole_ndc.x) + math::tau, math::tau);
    var angle1 : float =
                     fmod(atan2(-1.0 - epipole_ndc.y, 1.0 - epipole_ndc.x) + math::tau, math::tau);
    var angle2 : float =
                     fmod(atan2(1.0 - epipole_ndc.y, 1.0 - epipole_ndc.x) + math::tau, math::tau);
    var angle3 : float =
                     fmod(atan2(1.0 - epipole_ndc.y, -1.0 - epipole_ndc.x) + math::tau, math::tau);
    sort_epipolar_pair(angle0, angle1);
    sort_epipolar_pair(angle2, angle3);
    sort_epipolar_pair(angle0, angle2);
    sort_epipolar_pair(angle1, angle3);
    sort_epipolar_pair(angle1, angle2);

    let gaps =
        float4(angle1 - angle0, angle2 - angle1, angle3 - angle2, angle0 + math::tau - angle3);
    var largest_gap : float = gaps.x;
    result.start = angle1;
    if (gaps.y > largest_gap) {
        largest_gap = gaps.y;
        result.start = angle2;
    }
    if (gaps.z > largest_gap) {
        largest_gap = gaps.z;
        result.start = angle3;
    }
    if (gaps.w > largest_gap) {
        largest_gap = gaps.w;
        result.start = angle0;
    }
    result.span = max(math::tau - largest_gap, EPIPOLAR_EPSILON);
    return result;
}

public func epipolar_range_coordinate(range: EpipolarRange, epipole_ndc: float2, screen_ndc: float2)
    ->float {
    if (range.transverse)
        return saturate((intercept(range.axis, range.inverse_distance, screen_ndc) - range.start) /
                        range.span);
    let direction = screen_ndc - epipole_ndc;
    // 极点落在像素中心时，任一切片都从该点出发。
    let angle = atan2(direction.y, all(direction == 0.) ? 1. : direction.x);
    let offset = fmod(angle - range.start + math::tau, math::tau);
    if (offset <= range.span) {
        return offset / range.span;
    }
    // A corner can round to the opposite side of the circular seam. Snap only
    // to the nearer interval endpoint instead of stretching that error across
    // the complete angular domain.
    let distance_to_start = math::tau - offset;
    let distance_to_end = offset - range.span;
    return distance_to_start < distance_to_end ? 0.0 : 1.0;
}

public func epipolar_ray_segment(origin: float2, direction: float2)->EpipolarSegment {
    var entry_distance = 0.0;
    var exit_distance : float = EPIPOLAR_LIMIT * 4.0;
    var valid : bool = dot(direction, direction) > EPIPOLAR_EPSILON * EPIPOLAR_EPSILON;

    if (abs(direction.x) <= EPIPOLAR_EPSILON) {
        valid = valid && abs(origin.x) <= 1.0;
    } else {
        let distances = (float2(-1.0, 1.0) - origin.x) / direction.x;
        entry_distance = max(entry_distance, min(distances.x, distances.y));
        exit_distance = min(exit_distance, max(distances.x, distances.y));
    }
    if (abs(direction.y) <= EPIPOLAR_EPSILON) {
        valid = valid && abs(origin.y) <= 1.0;
    } else {
        let distances = (float2(-1.0, 1.0) - origin.y) / direction.y;
        entry_distance = max(entry_distance, min(distances.x, distances.y));
        exit_distance = min(exit_distance, max(distances.x, distances.y));
    }
    valid = valid && exit_distance >= entry_distance && exit_distance >= 0.0;

    var result : EpipolarSegment;
    result.entry = origin + direction * max(entry_distance, 0.0);
    result.exit = origin + direction * exit_distance;
    result.valid = valid;
    return result;
}

public func epipolar_slice_segment(epipole_ndc: float2, range: EpipolarRange, slice: int,
                                   slice_count: int)
    ->EpipolarSegment {
    let slice_coordinate = (float(slice) + 0.5) / float(slice_count);
    if (range.transverse) {
        let perpendicular = float2(-range.axis.y, range.axis.x);
        let offset = range.start + range.span * slice_coordinate;
        let direction = range.axis + perpendicular * (offset * range.inverse_distance);
        return epipolar_ray_segment(perpendicular * offset - direction * 4., direction);
    }
    let angle = range.start + range.span * slice_coordinate;
    var sine : float;
    var cosine : float;
    sincos(angle, sine, cosine);
    return epipolar_ray_segment(epipole_ndc, float2(cosine, sine));
}

public func epipolar_radial_coordinate(screen_ndc: float2, segment: EpipolarSegment)->float {
    let extent = segment.exit - segment.entry;
    let length_squared = dot(extent, extent);
    return length_squared > EPIPOLAR_EPSILON
               ? clamp(dot(screen_ndc - segment.entry, extent) / length_squared, 0.0, 1.0)
               : 0.0;
}

// 像素位于两条相邻极线之间。太阳在视锥内时跨越方位角接缝。
public func terrain_slices(ndc: float2, epipole: float2, range: EpipolarRange, count: int,
                           stride: int, offset: int, out fraction: float)
    ->int2 {
    let t = (epipolar_range_coordinate(range, epipole, ndc) * float(count) - 0.5 - float(offset)) /
            float(stride);
    let lower = int(floor(t));
    let last = count / stride - 1;
    fraction = frac(t);
    if (!range.transverse && range.span >= math::tau - EPIPOLAR_EPSILON) {
        return offset + stride * int2(lower < 0 ? last : lower, lower == last ? 0 : lower + 1);
    }
    return offset + stride * clamp(int2(lower, lower + 1), 0, last);
}
}
