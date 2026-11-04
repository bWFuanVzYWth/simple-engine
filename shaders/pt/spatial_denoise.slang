/// 单帧有限邻域重建；不包含时域历史、SDK 或显示变换。
module spatial_denoise;
import pt.denoise;

namespace pt
{
/// 同一输入像素的场景线性颜色与规范 guide；valid=false 的输入不参与邻域。
public struct SpatialSample
{
    public float3 color;
    public float depth;
    public float4 normal_roughness;
    public float3 diffuse;
    public float3 specular;
    public bool foreground;
    public bool valid;
};

/// 资源格式和存储由上层选择；每次 load 只读取指定像素，不调度或分配。
public interface ISpatialImage
{
    SpatialSample load(uint2 pixel);
};

/// 深度/法线/前景与材质边界保护；颜色不决定核权重，避免噪声中心自锁。
/// depth 为正向线性深度或 DENOISE_SKY_DEPTH；不跨前景/天空混合。
public float spatial_weight(float2 offset, SpatialSample center, SpatialSample sample)
{
    if (!sample.valid || !all(isfinite(sample.color)) || center.foreground != sample.foreground)
        return 0;
    bool sky = center.depth == DENOISE_SKY_DEPTH;
    if (sky != (sample.depth == DENOISE_SKY_DEPTH))
        return 0;
    float weight = exp(-dot(offset, offset) / 4);
    if (!sky)
    {
        if (!(center.depth > 0) || !(sample.depth > 0) || !isfinite(center.depth) ||
            !isfinite(sample.depth))
            return 0;
        float relative = (sample.depth - center.depth) / max(0.01, 0.02 * center.depth);
        float normal_length = dot(center.normal_roughness.xyz, center.normal_roughness.xyz) *
                              dot(sample.normal_roughness.xyz, sample.normal_roughness.xyz);
        if (!(normal_length > 1e-12) || !isfinite(normal_length))
            return 0;
        weight *= exp(-relative * relative) *
                  pow(saturate(dot(center.normal_roughness.xyz, sample.normal_roughness.xyz) *
                               rsqrt(normal_length)),
                      32);
    }
    float3 diffuse = center.diffuse - sample.diffuse;
    float3 specular = center.specular - sample.specular;
    float roughness = center.normal_roughness.w - sample.normal_roughness.w;
    weight *= exp(-64 * (dot(diffuse, diffuse) + dot(specular, specular) + roughness * roughness));
    return isfinite(weight) ? weight : 0;
}

/// 在连续输入像素坐标重建一个输出点，输入像素中心为整数，extent 两轴非零。
/// 固定 5x5 邻域、最多 25 次 load；边界只访问有效像素，不复制边缘样本。
/// 调用方选择输入/输出尺寸映射；无可用中心或邻居时 valid=false、color=0。
public float3 spatial_reconstruct<Image>(Image image, float2 source, uint2 extent, out bool valid)
    where Image : ISpatialImage
{
    int2 center_pixel = clamp(int2(floor(source + 0.5)), 0, int2(extent) - 1);
    SpatialSample center = image.load(uint2(center_pixel));
    valid = center.valid && all(isfinite(center.color));
    if (!valid)
        return 0;
    float3 sum = 0;
    float total = 0;
    for (int y = -2; y <= 2; ++y)
        for (int x = -2; x <= 2; ++x)
        {
            int2 pixel = center_pixel + int2(x, y);
            if (any(pixel < 0) || any(pixel >= int2(extent)))
                continue;
            SpatialSample sample = center;
            if (x != 0 || y != 0)
                sample = image.load(uint2(pixel));
            float weight = spatial_weight(float2(pixel) - source, center, sample);
            if (weight > 0)
            {
                sum += sample.color * weight;
                total += weight;
            }
        }
    valid = total > 0 && all(isfinite(sum));
    return valid ? sum / total : float3(0);
}
}
