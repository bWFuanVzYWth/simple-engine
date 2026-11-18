/// 单帧有限邻域重建；不包含时域历史、SDK 或显示变换。
module spatial;
import denoise.guide;

namespace denoise {
/// 同一输入像素的场景线性颜色与规范 guide；valid=false 的输入不参与邻域。
public struct SpatialSample {
    public var color : float3;
    public var depth : float;
    public var normal_roughness : float4;
    public var diffuse : float3;
    public var specular : float3;
    public var foreground : bool;
    public var valid : bool;
};

/// 资源格式和存储由上层选择；每次 load 只读取指定像素，不调度或分配。
public interface ISpatialImage {
    func load(pixel: uint2)->SpatialSample;
};

/// 深度/法线/前景与材质边界保护；颜色不决定核权重，避免噪声中心自锁。
/// depth 为正向线性深度或 DENOISE_SKY_DEPTH；不跨前景/天空混合。
func spatial_weight_prepared(offset: float2, center: SpatialSample, sample_value: SpatialSample,
                             inverse_depth_scale: float, center_normal_squared: float)
    ->float {

    if (!sample_value.valid || !all(isfinite(sample_value.color)) ||
        center.foreground != sample_value.foreground)
        return 0;
    let sky = center.depth == DENOISE_SKY_DEPTH;
    if (sky != (sample_value.depth == DENOISE_SKY_DEPTH))
        return 0;
    var exponent = -0.25 * dot(offset, offset);
    var angular = 1.0;
    if (!sky) {
        if (!(center.depth > 0) || !(sample_value.depth > 0) || !isfinite(center.depth) ||
            !isfinite(sample_value.depth))
            return 0;
        let relative = (sample_value.depth - center.depth) * inverse_depth_scale;
        let normal_length = center_normal_squared * dot(sample_value.normal_roughness.xyz,
                                                        sample_value.normal_roughness.xyz);
        if (!(normal_length > 1e-12) || !isfinite(normal_length))
            return 0;
        exponent -= relative * relative;
        let cosine = saturate(dot(center.normal_roughness.xyz, sample_value.normal_roughness.xyz) *
                              rsqrt(normal_length));
        let square = cosine * cosine;
        let fourth = square * square;
        let eighth = fourth * fourth;
        let sixteenth = eighth * eighth;
        angular = sixteenth * sixteenth;
    }
    let diffuse = center.diffuse - sample_value.diffuse;
    let specular = center.specular - sample_value.specular;
    let roughness = center.normal_roughness.w - sample_value.normal_roughness.w;
    exponent -= 64 * (dot(diffuse, diffuse) + dot(specular, specular) + roughness * roughness);
    let weight = exp(exponent) * angular;
    return isfinite(weight) ? weight : 0;
}

/// 同一深度、法线和材质权重；独立调用时在此准备中心的公共几何量。
/// depth为正线性深度或DENOISE_SKY_DEPTH；不跨前景/天空混合，颜色不决定权重。
public func spatial_weight(offset: float2, center: SpatialSample, sample_value: SpatialSample)
    ->float {
    return spatial_weight_prepared(offset, center, sample_value, 1 / max(0.01, 0.02 * center.depth),
                                   dot(center.normal_roughness.xyz, center.normal_roughness.xyz));
}

/// 在连续输入像素坐标重建一个输出点，输入像素中心为整数，extent 两轴非零。
/// 固定 5x5 邻域、最多 25 次 load；边界只访问有效像素，不复制边缘样本。
/// 调用方选择输入/输出尺寸映射；无可用中心或邻居时 valid=false、color=0。
public func spatial_reconstruct<Image>(image: Image, source: float2, extent: uint2, out valid: bool)
    ->float3
    where Image : ISpatialImage {
    let center_pixel = clamp(int2(floor(source + 0.5)), 0, int2(extent) - 1);
    let center : SpatialSample = image.load(uint2(center_pixel));
    valid = center.valid && all(isfinite(center.color));
    if (!valid)
        return 0;
    let inverse_depth_scale = 1 / max(0.01, 0.02 * center.depth);
    let center_normal_squared = dot(center.normal_roughness.xyz, center.normal_roughness.xyz);
    var sum : float3 = 0;
    var total : float = 0;
    for (var y: int = -2; y <= 2; ++y)
        for (var x: int = -2; x <= 2; ++x) {
            let pixel = center_pixel + int2(x, y);
            if (any(pixel < 0) || any(pixel >= int2(extent)))
                continue;
            var sample : SpatialSample = center;
            if (x != 0 || y != 0)
                sample = image.load(uint2(pixel));
            let weight = spatial_weight_prepared(float2(pixel) - source, center, sample,
                                                 inverse_depth_scale, center_normal_squared);
            if (weight > 0) {
                sum += sample.color * weight;
                total += weight;
            }
        }
    valid = total > 0 && all(isfinite(sum));
    return valid ? sum / total : float3(0);
}
}
