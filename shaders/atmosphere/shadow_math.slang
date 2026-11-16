module shadow_math;
namespace atmosphere
{
public static const let SUN_SHADOW_RESOLUTION = 512;
public func shadow_leaf_visible_length(ray_depth_at_zero: float, ray_depth_rate: float,
                                       start_distance: float, end_distance: float,
                                       blocker_depth: float, bias: float)
    ->float
{
    let start_depth = ray_depth_at_zero + ray_depth_rate * start_distance + bias;
    let end_depth = ray_depth_at_zero + ray_depth_rate * end_distance + bias;
    if (start_depth >= blocker_depth && end_depth >= blocker_depth)
    {
        return end_distance - start_distance;
    }
    if (start_depth < blocker_depth && end_depth < blocker_depth)
        return 0.0;
    let crossing = (blocker_depth - bias - ray_depth_at_zero) / ray_depth_rate;
    return ray_depth_rate > 0.0 ? end_distance - clamp(crossing, start_distance, end_distance)
                                : clamp(crossing, start_distance, end_distance) - start_distance;
}

public func shadow_texel_size(cascade: int)->float
{
    return cascade == 0 ? 0.5
                        : (cascade == 1 ? 1.0 : (cascade == 2 ? 2.0 : (cascade == 3 ? 4.0 : 8.0)));
}

}
