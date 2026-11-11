// 极线深度重建参考 Intel Outdoor Light Scattering (Copyright 2017 Intel Corporation, Apache-2.0)。
// 本实现使用相机到首命中的射线距离，在近场体积的径向采样前剔除跨深度边界的贡献。
module terrain_composite;
import atmosphere.sampling;
import atmosphere.terrain_frame;
import atmosphere.epipolar;
import color.primaries;
namespace atmosphere
{
struct TerrainFilter
{
    float2 radial;
    int2 slices;
    float2 weights;
};
TerrainFilter terrain_filter(TerrainFrame f, StructuredBuffer<uint> first_hits, float2 uv,
                             float metres, bool far_sample)
{
    TerrainFilter filter;
    float2 ndc = float2(2 * uv.x - 1, 1 - 2 * uv.y);
    AtmEpipolarRange range = terrain_range(f);
    float fraction;
    filter.slices = terrain_slices(ndc, f.projection.xy, range, TERRAIN_SLICES, fraction);
    filter.weights = float2(1. - fraction, fraction);
    for (int k = 0; k < 2; ++k)
    {
        int slice = filter.slices[k];
        AtmEpipolarSegment segment =
            atmEpipolarSliceSegment(f.projection.xy, range, slice, TERRAIN_SLICES);
        float x = atmEpipolarRadialCoordinate(ndc, segment) * float(TERRAIN_RADIAL) - 0.5;
        int left = int(floor(x));
        int2 lanes = clamp(int2(left, left + 1), 0, int(TERRAIN_RADIAL) - 1);
        uint base = uint(slice) * TERRAIN_RADIAL;
        float2 depth =
            asfloat(uint2(first_hits[base + lanes.x], first_hits[base + lanes.y]) & 0x7fffffffu);
        float2 relative = abs(depth - metres) / max(max(depth, metres), 1.0);
        // 平面内按双线性权重，深度不连续处用有限支撑，避免洞口像素给前景封闭墙面供光。
        float2 support =
            far_sample ? float2(depth >= TERRAIN_RANGE_M) : 1. - smoothstep(0.03, 0.06, relative);
        float2 weights = float2(1. - frac(x), frac(x)) * support;
        float sum = weights.x + weights.y;
        filter.radial[k] =
            (lerp(float(lanes.x), float(lanes.y), weights.y / max(sum, 1e-10)) + 0.5) /
            float(TERRAIN_RADIAL);
        filter.weights[k] *= segment.valid ? sum : 0;
    }
    float total = filter.weights.x + filter.weights.y;
    filter.weights = total >= 0.01 ? filter.weights / total : float2(0);
    return filter;
}
float4 terrain_sample(Texture3D<float4> volume, SamplerState sampler, TerrainFilter filter, float z)
{
    float4 sum = 0;
    for (int k = 0; k < 2; ++k)
        sum +=
            filter.weights[k] *
            volume.SampleLevel(sampler,
                               float3(filter.radial[k],
                                      (float(filter.slices[k]) + 0.5) / float(TERRAIN_SLICES), z),
                               0);
    return sum;
}
/// 在相机空气段合成线性 BT.709。有限命中直接使用受光积分；无支持的采样仅保留背景透射。
public float3 composite_terrain_bt709(Texture3D<float4> clear_light,
                                      Texture3D<float4> transmittance, Texture3D<float4> volume,
                                      StructuredBuffer<uint> first_hits, SamplerState sampler,
                                      TerrainFrame frame, float2 uv, float metres,
                                      float max_distance_km, bool surface, float3 background)
{
    if (metres <= 0)
        return background;
    if (frame.source.state.y == 0)
    {
        if (!surface)
            return background;
        AerialSample air =
            aerial_sample(clear_light, transmittance, sampler, uv, metres * 0.001, max_distance_km);
        return color::rec2020_to_bt709(composite_aerial(air, color::bt709_to_rec2020(background)));
    }
    bool beyond = !surface || metres > TERRAIN_RANGE_M;
    TerrainFilter filter =
        terrain_filter(frame, first_hits, uv, min(metres, TERRAIN_RANGE_M), beyond);
    float layer_count = float(TERRAIN_DEPTH + 1u);
    float4 boundary = 0;
    if (beyond)
        boundary =
            terrain_sample(volume, sampler, filter, (float(TERRAIN_DEPTH) + 0.5) / layer_count);
    if (!surface)
        return background - color::rec2020_to_bt709(boundary.xyz);
    AerialSample clear =
        aerial_sample(clear_light, transmittance, sampler, uv, metres * 0.001, max_distance_km);
    // 介质积分在实际距离上插值。零距离为解析的零，最后一个普通深度节点正好到覆盖边界。
    float squared = saturate(metres / TERRAIN_RANGE_M) * float(TERRAIN_DEPTH * TERRAIN_DEPTH);
    float lower = min(floor(sqrt(squared)), float(TERRAIN_DEPTH - 1u));
    float node = lower + (squared - lower * lower) / (2. * lower + 1.);
    float z = (max(node, 1.) - 0.5) / layer_count;
    float3 radiance = terrain_sample(volume, sampler, filter, z).xyz * saturate(node);
    if (beyond)
    {
        AerialSample near_clear = aerial_sample(clear_light, transmittance, sampler, uv,
                                                TERRAIN_RANGE_M * 0.001, max_distance_km);
        // 五级网格之外按最后有覆盖的区间延续可见度，封闭洞穴不会重新得到无条件的远场晴空散射。
        radiance += max(clear.radiance - near_clear.radiance, 0) * saturate(boundary.w);
    }
    return color::rec2020_to_bt709(radiance +
                                   clear.transmittance * color::bt709_to_rec2020(background));
}
}
