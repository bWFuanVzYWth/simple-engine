// 极线参数化与相邻切片重建移植自 Prime PT e0260fc4。
module epipolar;
namespace atmosphere
{
float2 primeCanonicalUvToClip(float2 uv)
{
    return float2(2.0 * uv.x - 1.0, 1.0 - 2.0 * uv.y);
}

public static const float ATM_EPIPOLAR_LIMIT = 16384.0;
public static const float ATM_EPIPOLAR_EPSILON = 1.0e-6;

public struct AtmEpipolarSegment
{
    public float2 entry;
    public float2 exit;
    public bool valid;
};

public struct AtmEpipolarRange
{
    public float start;
    public float span;
    public float2 axis;
    public float extent;
    public bool parallel;
};

public void atmEpipolarSortPair(inout float first, inout float second)
{
    if (first > second)
    {
        float temporary = first;
        first = second;
        second = temporary;
    }
}

public AtmEpipolarRange atmEpipolarVisibleRange(float2 epipoleNdc)
{
    const float TWO_PI = 6.28318530717958647692;
    AtmEpipolarRange result;
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
        float2 perpendicular = float2(-result.axis.y, result.axis.x);
        result.extent = abs(perpendicular.x) + abs(perpendicular.y);
        result.parallel = true;
        return result;
    }

    float angle0 = fmod(atan2(-1.0 - epipoleNdc.y, -1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    float angle1 = fmod(atan2(-1.0 - epipoleNdc.y, 1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    float angle2 = fmod(atan2(1.0 - epipoleNdc.y, 1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    float angle3 = fmod(atan2(1.0 - epipoleNdc.y, -1.0 - epipoleNdc.x) + TWO_PI, TWO_PI);
    atmEpipolarSortPair(angle0, angle1);
    atmEpipolarSortPair(angle2, angle3);
    atmEpipolarSortPair(angle0, angle2);
    atmEpipolarSortPair(angle1, angle3);
    atmEpipolarSortPair(angle1, angle2);

    float4 gaps =
        float4(angle1 - angle0, angle2 - angle1, angle3 - angle2, angle0 + TWO_PI - angle3);
    float largestGap = gaps.x;
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

public float atmEpipolarRangeCoordinate(AtmEpipolarRange range, float2 epipoleNdc, float2 screenNdc)
{
    if (range.parallel)
    {
        float2 perpendicular = float2(-range.axis.y, range.axis.x);
        return clamp(dot(screenNdc, perpendicular) / max(2.0 * range.extent, ATM_EPIPOLAR_EPSILON) +
                         0.5,
                     0.0, 1.0);
    }
    const float TWO_PI = 6.28318530717958647692;
    float2 direction = screenNdc - epipoleNdc;
    float angle = atan2(direction.y, direction.x);
    float offset = fmod(angle - range.start + TWO_PI, TWO_PI);
    if (offset <= range.span)
    {
        return offset / range.span;
    }
    // A corner can round to the opposite side of the circular seam. Snap only
    // to the nearer interval endpoint instead of stretching that error across
    // the complete angular domain.
    float distanceToStart = TWO_PI - offset;
    float distanceToEnd = offset - range.span;
    return distanceToStart < distanceToEnd ? 0.0 : 1.0;
}

public AtmEpipolarSegment atmEpipolarRaySegment(float2 origin, float2 direction)
{
    float entryDistance = 0.0;
    float exitDistance = ATM_EPIPOLAR_LIMIT * 4.0;
    bool valid = dot(direction, direction) > ATM_EPIPOLAR_EPSILON * ATM_EPIPOLAR_EPSILON;

    if (abs(direction.x) <= ATM_EPIPOLAR_EPSILON)
    {
        valid = valid && abs(origin.x) <= 1.0;
    }
    else
    {
        float2 distances = (float2(-1.0, 1.0) - origin.x) / direction.x;
        entryDistance = max(entryDistance, min(distances.x, distances.y));
        exitDistance = min(exitDistance, max(distances.x, distances.y));
    }
    if (abs(direction.y) <= ATM_EPIPOLAR_EPSILON)
    {
        valid = valid && abs(origin.y) <= 1.0;
    }
    else
    {
        float2 distances = (float2(-1.0, 1.0) - origin.y) / direction.y;
        entryDistance = max(entryDistance, min(distances.x, distances.y));
        exitDistance = min(exitDistance, max(distances.x, distances.y));
    }
    valid = valid && exitDistance >= entryDistance && exitDistance >= 0.0;

    AtmEpipolarSegment result;
    result.entry = origin + direction * max(entryDistance, 0.0);
    result.exit = origin + direction * exitDistance;
    result.valid = valid;
    return result;
}

public AtmEpipolarSegment atmEpipolarSliceSegment(float2 epipoleNdc, AtmEpipolarRange range,
                                                  int slice, int sliceCount)
{
    float sliceCoordinate = (float(slice) + 0.5) / float(sliceCount);
    if (range.parallel)
    {
        float2 perpendicular = float2(-range.axis.y, range.axis.x);
        float offset = lerp(-range.extent, range.extent, sliceCoordinate);
        float2 origin = perpendicular * offset - range.axis * 4.0;
        return atmEpipolarRaySegment(origin, range.axis);
    }
    float angle = range.start + range.span * sliceCoordinate;
    return atmEpipolarRaySegment(epipoleNdc, float2(cos(angle), sin(angle)));
}

public float atmEpipolarRadialCoordinate(float2 screenNdc, AtmEpipolarSegment segment)
{
    float2 extent = segment.exit - segment.entry;
    float lengthSquared = dot(extent, extent);
    return lengthSquared > ATM_EPIPOLAR_EPSILON
               ? clamp(dot(screenNdc - segment.entry, extent) / lengthSquared, 0.0, 1.0)
               : 0.0;
}

// 像素位于两条相邻极线之间。太阳在视锥内时跨越方位角接缝。
public int2 terrain_slices(float2 ndc, float2 epipole, AtmEpipolarRange range, int count,
                           out float fraction)
{
    float t = atmEpipolarRangeCoordinate(range, epipole, ndc) * float(count) - 0.5;
    int lower = int(floor(t));
    fraction = frac(t);
    if (!range.parallel && range.span >= 6.283185307179586 - ATM_EPIPOLAR_EPSILON)
    {
        int i = (lower % count + count) % count;
        return int2(i, (i + 1) % count);
    }
    return clamp(int2(lower, lower + 1), 0, count - 1);
}
}
