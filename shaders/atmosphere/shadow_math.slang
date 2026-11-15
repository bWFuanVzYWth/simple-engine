module shadow_math;
namespace atmosphere
{
public static const let PRIME_SUN_SHADOW_RESOLUTION = 512;
public func primeSunShadowLeafVisibleLength(rayDepthAtZero: float, rayDepthRate: float,
                                            startDistance: float, endDistance: float,
                                            blockerDepth: float, bias: float)
    ->float
{
    let startDepth : float = rayDepthAtZero + rayDepthRate * startDistance + bias;
    let endDepth : float = rayDepthAtZero + rayDepthRate * endDistance + bias;
    if (startDepth >= blockerDepth && endDepth >= blockerDepth)
    {
        return endDistance - startDistance;
    }
    if (startDepth < blockerDepth && endDepth < blockerDepth)
        return 0.0;
    let crossing : float = (blockerDepth - bias - rayDepthAtZero) / rayDepthRate;
    return rayDepthRate > 0.0 ? endDistance - clamp(crossing, startDistance, endDistance)
                              : clamp(crossing, startDistance, endDistance) - startDistance;
}

public func sunShadowTexelSize(cascade: int)->float
{
    return cascade == 0 ? 0.5
                        : (cascade == 1 ? 1.0 : (cascade == 2 ? 2.0 : (cascade == 3 ? 4.0 : 8.0)));
}

}
