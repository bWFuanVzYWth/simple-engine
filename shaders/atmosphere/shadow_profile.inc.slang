// Entry-local profile workspace. Original Prime/Intel profile algorithm.
// Copyright 2017 Intel Corporation; Apache-2.0. See THIRD_PARTY_NOTICES.md.
import atmosphere.shadow_math;
using namespace atmosphere;
public static const let SUN_SHADOW_CASCADE_COUNT = 5;
public static const let SUN_SHADOW_MAX_DISTANCE_METERS = 2048.0;
public static const let SUN_SHADOW_DEPTH_BIAS_METERS = 0.02;
public static const let SUN_SHADOW_CASCADE_RADIUS_TEXELS = 254.0;
public static const let SUN_SHADOW_TRAVERSAL_EPSILON = 1.0e-5;

public interface IPrimeSunShadowProfileSource
{
    func texelSize(cascade: int)->float;
    func blockerDepth(cascade: int, worldNode: int2)->float;
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

public groupshared var sunShadowProfileBasisU : float3;
public groupshared var sunShadowProfileBasisV : float3;
public groupshared var sunShadowProfileDirectionToSun : float3;
public groupshared var sunShadowProfileCamera : float2;
public groupshared var sunShadowProfileDirection : float2;
public groupshared var sunShadowProfileDirectionMax : float;
public groupshared var sunShadowProfileCameraDepth : float;
public groupshared var sunShadowProfileCounts : int[SUN_SHADOW_CASCADE_COUNT];
public groupshared var sunShadowProfileEntries : float2[SUN_SHADOW_PROFILE_TOTAL_CAPACITY];

public func primeInitializeSunShadowProfile(directionToSun: float3, projectedCamera: float2,
                                            cameraDepth: float, edgeDirection: float3,
                                            basisU: float3, basisV: float3)
    ->void
{
    sunShadowProfileDirectionToSun = normalize(directionToSun);
    sunShadowProfileBasisU = basisU;
    sunShadowProfileBasisV = basisV;
    sunShadowProfileCamera = projectedCamera;
    sunShadowProfileCameraDepth = cameraDepth;
    let projectedEdge = float2(dot(edgeDirection, sunShadowProfileBasisU),
                               dot(edgeDirection, sunShadowProfileBasisV));
    let projectedLength : float = length(projectedEdge);
    sunShadowProfileDirection =
        projectedLength > 1.0e-6 ? projectedEdge / projectedLength : float2(1.0, 0.0);
    sunShadowProfileDirectionMax =
        max(abs(sunShadowProfileDirection.x), abs(sunShadowProfileDirection.y));
}

func sunShadowProfileBase(cascade: int)->int
{
    return cascade == 0
               ? 0
               : SUN_SHADOW_PROFILE_NEAR_CAPACITY + (cascade - 1) * SUN_SHADOW_PROFILE_FAR_CAPACITY;
}

func sunShadowProfileCapacity(cascade: int)->int
{
    return cascade == 0 ? SUN_SHADOW_PROFILE_NEAR_CAPACITY : SUN_SHADOW_PROFILE_FAR_CAPACITY;
}

public func sunShadowProfileCascadeEnd<Source>(source: Source, cascade: int)->float
    where Source : IPrimeSunShadowProfileSource
{
    let radius : float = SUN_SHADOW_CASCADE_RADIUS_TEXELS * source.texelSize(cascade);
    return min(radius / max(sunShadowProfileDirectionMax, SUN_SHADOW_PROFILE_EPSILON),
               SUN_SHADOW_MAX_DISTANCE_METERS);
}

public func sunShadowBuildProfileCascade<Source>(source: Source, cascade: int)->void
    where Source : IPrimeSunShadowProfileSource
{
    let startScalar : float = cascade == 0 ? 0.0 : sunShadowProfileCascadeEnd(source, cascade - 1);
    let endScalar : float = sunShadowProfileCascadeEnd(source, cascade);
    if (endScalar <= startScalar + SUN_SHADOW_PROFILE_EPSILON)
    {
        sunShadowProfileCounts[cascade] = 0;
        return;
    }

    let texelSize : float = source.texelSize(cascade);
    let gridRate : float2 = sunShadowProfileDirection / texelSize;
    // Only the toroidal storage coordinate matters. Keeping DDA coordinates
    // within one clipmap period prevents the directional boundary offset from
    // disappearing into the ULP of a large world-space grid coordinate.
    let absoluteGridOrigin : float2 = sunShadowProfileCamera / texelSize;
    let gridPoint : float2 = fmod(absoluteGridOrigin + gridRate * startScalar,
                                  float(PRIME_SUN_SHADOW_RESOLUTION));
    var cell = int2(floor(gridPoint + sign(gridRate) * SUN_SHADOW_PROFILE_EPSILON));
    let cellStep = int2(sign(gridRate));
    var nextCrossing = float2(1.0e20);
    var crossingStep = float2(1.0e20);
    if (gridRate.x > 0.0)
    {
        nextCrossing.x = (float(cell.x + 1) - gridPoint.x) / gridRate.x;
        crossingStep.x = 1.0 / gridRate.x;
    }
    else if (gridRate.x < 0.0)
    {
        nextCrossing.x = (float(cell.x) - gridPoint.x) / gridRate.x;
        crossingStep.x = -1.0 / gridRate.x;
    }
    if (gridRate.y > 0.0)
    {
        nextCrossing.y = (float(cell.y + 1) - gridPoint.y) / gridRate.y;
        crossingStep.y = 1.0 / gridRate.y;
    }
    else if (gridRate.y < 0.0)
    {
        nextCrossing.y = (float(cell.y) - gridPoint.y) / gridRate.y;
        crossingStep.y = -1.0 / gridRate.y;
    }
    let profileLength : float = endScalar - startScalar;
    var cursor = 0.0;
    var count = 0;
    let base : int = sunShadowProfileBase(cascade);
    let capacity : int = sunShadowProfileCapacity(cascade);

    for (; count < capacity; ++count)
    {
        let spanEnd : float = min(profileLength, min(nextCrossing.x, nextCrossing.y));
        if (spanEnd <= cursor + SUN_SHADOW_PROFILE_EPSILON)
        {
            sunShadowProfileCounts[cascade] = -1;
            return;
        }

        let resolvedShadow : float = source.blockerDepth(cascade, cell);
        sunShadowProfileEntries[base + count] = float2(startScalar + spanEnd, resolvedShadow);
        cursor = spanEnd;
        if (cursor + SUN_SHADOW_PROFILE_EPSILON >= profileLength)
        {
            sunShadowProfileCounts[cascade] = count + 1;
            return;
        }

        let crossesX : bool = nextCrossing.x <= spanEnd + SUN_SHADOW_PROFILE_EPSILON;
        let crossesY : bool = nextCrossing.y <= spanEnd + SUN_SHADOW_PROFILE_EPSILON;
        if (!crossesX && !crossesY)
        {
            sunShadowProfileCounts[cascade] = -1;
            return;
        }
        if (crossesX)
        {
            cell.x += cellStep.x;
            nextCrossing.x += crossingStep.x;
        }
        if (crossesY)
        {
            cell.y += cellStep.y;
            nextCrossing.y += crossingStep.y;
        }
    }

    // A truncated profile is unknown, never lit.
    sunShadowProfileCounts[cascade] = -1;
}

func sunShadowProfileFind(cascade: int, scalar: float)->int
{
    let count : int = sunShadowProfileCounts[cascade];
    let base : int = sunShadowProfileBase(cascade);
    var low = 0;
    var high : int = count;
    while (low < high)
    {
        let middle : int = (low + high) >> 1;
        if (sunShadowProfileEntries[base + middle].x <= scalar + SUN_SHADOW_PROFILE_EPSILON)
        {
            low = middle + 1;
        }
        else
        {
            high = middle;
        }
    }
    return low;
}

func sunShadowProfileCascadeLitLength(cascade: int, startDistance: float, endDistance: float,
                                      scalarRate: float, rayDepthRate: float)
    ->float
{
    let count : int = sunShadowProfileCounts[cascade];
    if (count <= 0 || endDistance <= startDistance)
    {
        return 0.0;
    }

    let base : int = sunShadowProfileBase(cascade);
    if (scalarRate <= SUN_SHADOW_PROFILE_EPSILON)
    {
        return primeSunShadowLeafVisibleLength(
            sunShadowProfileCameraDepth, rayDepthRate, startDistance, endDistance,
            sunShadowProfileEntries[base].y, SUN_SHADOW_DEPTH_BIAS_METERS);
    }

    let startScalar : float = scalarRate * startDistance;
    var index : int = sunShadowProfileFind(cascade, startScalar);
    var cursor : float = startDistance;
    var litLength = 0.0;
    let capacity : int = sunShadowProfileCapacity(cascade);
    for (var traversal = 0; traversal < capacity && index < count &&
                            cursor + SUN_SHADOW_TRAVERSAL_EPSILON < endDistance;
         ++traversal, ++index)
    {
        let entry : float2 = sunShadowProfileEntries[base + index];
        let spanEnd : float = min(entry.x / scalarRate, endDistance);
        if (spanEnd <= cursor + SUN_SHADOW_TRAVERSAL_EPSILON)
        {
            continue;
        }
        litLength +=
            primeSunShadowLeafVisibleLength(sunShadowProfileCameraDepth, rayDepthRate, cursor,
                                            spanEnd, entry.y, SUN_SHADOW_DEPTH_BIAS_METERS);
        cursor = spanEnd;
    }
    // Any tail not represented by the profile is intentionally shadowed.
    return litLength;
}

public func localSunVisibilityInterval<Source>(source: Source, rayDirection: float3,
                                               startDistance: float, endDistance: float)
    ->float
    where Source : IPrimeSunShadowProfileSource
{
    if (endDistance <= startDistance)
    {
        return 1.0;
    }
    let tracedEnd : float = min(endDistance, SUN_SHADOW_MAX_DISTANCE_METERS);
    if (tracedEnd <= startDistance)
    {
        return 0.0;
    }

    let projectedRay = float2(dot(rayDirection, sunShadowProfileBasisU),
                              dot(rayDirection, sunShadowProfileBasisV));
    var scalarRate : float = dot(projectedRay, sunShadowProfileDirection);
    let perpendicularRate : float = abs(projectedRay.x * sunShadowProfileDirection.y -
                                        projectedRay.y * sunShadowProfileDirection.x);
    if (scalarRate < -SUN_SHADOW_PROFILE_EPSILON || perpendicularRate > 2.0e-4)
    {
        return 0.0;
    }
    scalarRate = max(scalarRate, 0.0);
    let rayDepthRate : float = dot(rayDirection, sunShadowProfileDirectionToSun);

    var cursor : float = startDistance;
    var litLength = 0.0;
    for (var cascade = 0; cascade < SUN_SHADOW_CASCADE_COUNT && cursor < tracedEnd; ++cascade)
    {
        let cascadeEnd
            : float = scalarRate > SUN_SHADOW_PROFILE_EPSILON
                          ? min(tracedEnd, sunShadowProfileCascadeEnd(source, cascade) / scalarRate)
                          : tracedEnd;
        if (cascadeEnd > cursor)
        {
            litLength += sunShadowProfileCascadeLitLength(cascade, cursor, cascadeEnd, scalarRate,
                                                          rayDepthRate);
            cursor = cascadeEnd;
        }
    }
    return clamp(litLength / (endDistance - startDistance), 0.0, 1.0);
}
