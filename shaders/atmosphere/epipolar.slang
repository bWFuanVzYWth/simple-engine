// 极线参数化与相邻切片重建移植自 Prime PT e0260fc4。
module epipolar;
namespace atmosphere
{
func primeCanonicalUvToClip(uv: float2)->float2
{
    return float2(2.0 * uv.x - 1.0, 1.0 - 2.0 * uv.y);
}

public static const let ATM_EPIPOLAR_LIMIT = 16384.0;
public static const let ATM_EPIPOLAR_EPSILON = 1.0e-6;

public struct AtmEpipolarSegment
{
    public var entry : float2;
    public var exit : float2;
    public var valid : bool;
};

public struct AtmEpipolarRange
{
    public var start : float;
    public var span : float;
    public var axis : float2;
    public var extent : float;
    public var parallel : bool;
};

public func atmEpipolarSortPair(inout first: float, inout second: float)->void
{
    if (first > second)
    {
        let temporary : float = first;
        first = second;
        second = temporary;
    }
}

public func atmEpipolarVisibleRange(epipoleNdc: float2)->AtmEpipolarRange
{
    let TWO_PI = 6.28318530717958647692;
    var result : AtmEpipolarRange;
    result.axis = float2(0.0);
    result.extent = 0.0;
    result.parallel = false;
    if (all(abs(epipoleNdc) <= float2(1.0)))
    {
        result.start = -3.14159265358979323846;
        result.span = TWO_PI;
        return result;
    }
    // Beyond this distance the finite epipole differs from parallel lines by
    // less than a pixel, while subtracting two f32 angles becomes less stable.
    if (max(abs(epipoleNdc.x), abs(epipoleNdc.y)) > 2048.0)
    {
        result.start = 0.0;
        result.span = 1.0;
        result.axis = normalize(-epipoleNdc);
        let perpendicular = float2(-result.axis.y, result.axis.x);
        result.extent = abs(perpendicular.x) + abs(perpendicular.y);
        result.parallel = true;
        return result;
    }

    var angle0 : float = fmod(atan2(-1.0 - epipoleNdc.y, -1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    var angle1 : float = fmod(atan2(-1.0 - epipoleNdc.y, 1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    var angle2 : float = fmod(atan2(1.0 - epipoleNdc.y, 1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    var angle3 : float = fmod(atan2(1.0 - epipoleNdc.y, -1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    atmEpipolarSortPair(angle0, angle1);
    atmEpipolarSortPair(angle2, angle3);
    atmEpipolarSortPair(angle0, angle2);
    atmEpipolarSortPair(angle1, angle3);
    atmEpipolarSortPair(angle1, angle2);

    let gaps = float4(angle1 - angle0, angle2 - angle1, angle3 - angle2, angle0 + TWO_PI - angle3);
    var largestGap : float = gaps.x;
    result.start = angle1;
    if (gaps.y > largestGap)
    {
        largestGap = gaps.y;
        result.start = angle2;
    }
    if (gaps.z > largestGap)
    {
        largestGap = gaps.z;
        result.start = angle3;
    }
    if (gaps.w > largestGap)
    {
        largestGap = gaps.w;
        result.start = angle0;
    }
    result.span = max(TWO_PI - largestGap, ATM_EPIPOLAR_EPSILON);
    return result;
}

public func atmEpipolarRangeCoordinate(range: AtmEpipolarRange, epipoleNdc: float2,
                                       screenNdc: float2)
    ->float
{
    if (range.parallel)
    {
        let perpendicular = float2(-range.axis.y, range.axis.x);
        return clamp(dot(screenNdc, perpendicular) / max(2.0 * range.extent, ATM_EPIPOLAR_EPSILON) +
                         0.5,
                     0.0, 1.0);
    }
    let TWO_PI = 6.28318530717958647692;
    let direction : float2 = screenNdc - epipoleNdc;
    let angle : float = atan2(direction.y, direction.x);
    let offset : float = fmod(angle - range.start + TWO_PI, TWO_PI);
    if (offset <= range.span)
    {
        return offset / range.span;
    }
    // A corner can round to the opposite side of the circular seam. Snap only
    // to the nearer interval endpoint instead of stretching that error across
    // the complete angular domain.
    let distanceToStart : float = TWO_PI - offset;
    let distanceToEnd : float = offset - range.span;
    return distanceToStart < distanceToEnd ? 0.0 : 1.0;
}

public func atmEpipolarRaySegment(origin: float2, direction: float2)->AtmEpipolarSegment
{
    var entryDistance = 0.0;
    var exitDistance : float = ATM_EPIPOLAR_LIMIT * 4.0;
    var valid : bool = dot(direction, direction) > ATM_EPIPOLAR_EPSILON * ATM_EPIPOLAR_EPSILON;

    if (abs(direction.x) <= ATM_EPIPOLAR_EPSILON)
    {
        valid = valid && abs(origin.x) <= 1.0;
    }
    else
    {
        let distances : float2 = (float2(-1.0, 1.0) - origin.x) / direction.x;
        entryDistance = max(entryDistance, min(distances.x, distances.y));
        exitDistance = min(exitDistance, max(distances.x, distances.y));
    }
    if (abs(direction.y) <= ATM_EPIPOLAR_EPSILON)
    {
        valid = valid && abs(origin.y) <= 1.0;
    }
    else
    {
        let distances : float2 = (float2(-1.0, 1.0) - origin.y) / direction.y;
        entryDistance = max(entryDistance, min(distances.x, distances.y));
        exitDistance = min(exitDistance, max(distances.x, distances.y));
    }
    valid = valid && exitDistance >= entryDistance && exitDistance >= 0.0;

    var result : AtmEpipolarSegment;
    result.entry = origin + direction * max(entryDistance, 0.0);
    result.exit = origin + direction * exitDistance;
    result.valid = valid;
    return result;
}

public func atmEpipolarSliceSegment(epipoleNdc: float2, range: AtmEpipolarRange, slice: int,
                                    sliceCount: int)
    ->AtmEpipolarSegment
{
    let sliceCoordinate : float = (float(slice) + 0.5) / float(sliceCount);
    if (range.parallel)
    {
        let perpendicular = float2(-range.axis.y, range.axis.x);
        let offset : float = lerp(-range.extent, range.extent, sliceCoordinate);
        let origin : float2 = perpendicular * offset - range.axis * 4.0;
        return atmEpipolarRaySegment(origin, range.axis);
    }
    let angle : float = range.start + range.span * sliceCoordinate;
    var sine : float;
    var cosine : float;
    sincos(angle, sine, cosine);
    return atmEpipolarRaySegment(epipoleNdc, float2(cosine, sine));
}

public func atmEpipolarRadialCoordinate(screenNdc: float2, segment: AtmEpipolarSegment)->float
{
    let extent : float2 = segment.exit - segment.entry;
    let lengthSquared : float = dot(extent, extent);
    return lengthSquared > ATM_EPIPOLAR_EPSILON
               ? clamp(dot(screenNdc - segment.entry, extent) / lengthSquared, 0.0, 1.0)
               : 0.0;
}

// 像素位于两条相邻极线之间。太阳在视锥内时跨越方位角接缝。
public func terrain_slices(ndc: float2, epipole: float2, range: AtmEpipolarRange, count: int,
                           out fraction: float)
    ->int2
{
    let t : float = atmEpipolarRangeCoordinate(range, epipole, ndc) * float(count) - 0.5;
    let lower = int(floor(t));
    fraction = frac(t);
    if (!range.parallel && range.span >= 6.283185307179586 - ATM_EPIPOLAR_EPSILON)
    {
        let i : int = (lower % count + count) % count;
        return int2(i, (i + 1) % count);
    }
    return clamp(int2(lower, lower + 1), 0, count - 1);
}
}
