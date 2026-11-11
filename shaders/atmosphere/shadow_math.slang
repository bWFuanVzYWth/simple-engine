module shadow_math;
namespace atmosphere
{
public static const int PRIME_SUN_SHADOW_RESOLUTION = 512;
public float primeSunShadowLeafVisibleLength(float rayDepthAtZero, float rayDepthRate,
                                             float startDistance, float endDistance,
                                             float blockerDepth, float bias)
{
    float startDepth = rayDepthAtZero + rayDepthRate * startDistance + bias;
    float endDepth = rayDepthAtZero + rayDepthRate * endDistance + bias;
    if (startDepth >= blockerDepth && endDepth >= blockerDepth)
    {
        return endDistance - startDistance;
    }
    if (startDepth < blockerDepth && endDepth < blockerDepth)
        return 0.0;
    float crossing = (blockerDepth - bias - rayDepthAtZero) / rayDepthRate;
    return rayDepthRate > 0.0 ? endDistance - clamp(crossing, startDistance, endDistance)
                              : clamp(crossing, startDistance, endDistance) - startDistance;
}

public float sunShadowTexelSize(int cascade)
{
    return cascade == 0 ? 0.5
                        : (cascade == 1 ? 1.0 : (cascade == 2 ? 2.0 : (cascade == 3 ? 4.0 : 8.0)));
}

}
