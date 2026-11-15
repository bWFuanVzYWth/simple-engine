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
    var radial : float2;
    var slices : int2;
    var weights : float2;
};
func terrain_filter(f: TerrainFrame, first_hits: StructuredBuffer<uint>, uv: float2, metres: float,
                    far_sample: bool)
    ->TerrainFilter
{
    var filter : TerrainFilter;
    let ndc = float2(2 * uv.x - 1, 1 - 2 * uv.y);
    let range : AtmEpipolarRange = terrain_range(f);
    var fraction : float;
    filter.slices = terrain_slices(ndc, f.projection.xy, range, TERRAIN_SLICES, fraction);
    filter.weights = float2(1. - fraction, fraction);
    for (var k = 0; k < 2; ++k)
    {
        let slice : int = filter.slices[k];
        let segment : AtmEpipolarSegment =
                          atmEpipolarSliceSegment(f.projection.xy, range, slice, TERRAIN_SLICES);
        let x : float = atmEpipolarRadialCoordinate(ndc, segment) * float(TERRAIN_RADIAL) - 0.5;
        let left = int(floor(x));
        let lanes : int2 = clamp(int2(left, left + 1), 0, int(TERRAIN_RADIAL) - 1);
        let base : uint = uint(slice) * TERRAIN_RADIAL;
        let depth : float2 = asfloat(uint2(first_hits[base + lanes.x], first_hits[base + lanes.y]) &
                                     0x7fffffffu);
        let relative : float2 = abs(depth - metres) / max(max(depth, metres), 1.0);
        // 平面内按双线性权重，深度不连续处用有限支撑，避免洞口像素给前景封闭墙面供光。
        let support : float2 = far_sample ? float2(depth >= TERRAIN_RANGE_M)
                                          : 1. - smoothstep(0.03, 0.06, relative);
        var weights : float2 = float2(1. - frac(x), frac(x)) * support;
        let sum : float = weights.x + weights.y;
        filter.radial[k] =
            (lerp(float(lanes.x), float(lanes.y), weights.y / max(sum, 1e-10)) + 0.5) /
            float(TERRAIN_RADIAL);
        filter.weights[k] *= segment.valid ? sum : 0;
    }
    let total : float = filter.weights.x + filter.weights.y;
    filter.weights = total >= 0.01 ? filter.weights / total : float2(0);
    return filter;
}
func terrain_sample(volume: Texture3D<float4>, sampler: SamplerState, filter: TerrainFilter,
                    z: float)
    ->float4
{
    var sum : float4 = 0;
    for (var k = 0; k < 2; ++k)
        sum +=
            filter.weights[k] *
            volume.SampleLevel(sampler,
                               float3(filter.radial[k],
                                      (float(filter.slices[k]) + 0.5) / float(TERRAIN_SLICES), z),
                               0);
    return sum;
}
/// 在相机空气段合成线性 BT.709。有限命中直接使用受光积分；无支持的采样仅保留背景透射。
public func composite_terrain_bt709(clear_light: Texture3D<float4>,
                                    transmittance: Texture3D<float4>, volume: Texture3D<float4>,
                                    first_hits: StructuredBuffer<uint>, sampler: SamplerState,
                                    frame: TerrainFrame, uv: float2, metres: float,
                                    max_distance_km: float, surface: bool, background: float3)
    ->float3
{
    if (metres <= 0)
        return background;
    if (frame.source.state.y == 0)
    {
        if (!surface)
            return background;
        let air : AerialSample = aerial_sample(clear_light, transmittance, sampler, uv,
                                               metres * 0.001, max_distance_km);
        return color::rec2020_to_bt709(composite_aerial(air, color::bt709_to_rec2020(background)));
    }
    let beyond : bool = !surface || metres > TERRAIN_RANGE_M;
    let filter : TerrainFilter =
                     terrain_filter(frame, first_hits, uv, min(metres, TERRAIN_RANGE_M), beyond);
    let layer_count = float(TERRAIN_DEPTH + 1u);
    var boundary : float4 = 0;
    if (beyond)
        boundary =
            terrain_sample(volume, sampler, filter, (float(TERRAIN_DEPTH) + 0.5) / layer_count);
    if (!surface)
        return background - color::rec2020_to_bt709(boundary.xyz);
    let clear : AerialSample = aerial_sample(clear_light, transmittance, sampler, uv,
                                             metres * 0.001, max_distance_km);
    // 介质积分在实际距离上插值。零距离为解析的零，最后一个普通深度节点正好到覆盖边界。
    let squared : float = saturate(metres / TERRAIN_RANGE_M) * float(TERRAIN_DEPTH * TERRAIN_DEPTH);
    let lower : float = min(floor(sqrt(squared)), float(TERRAIN_DEPTH - 1u));
    let node : float = lower + (squared - lower * lower) / (2. * lower + 1.);
    let z : float = (max(node, 1.) - 0.5) / layer_count;
    var radiance : float3 = terrain_sample(volume, sampler, filter, z).xyz * saturate(node);
    if (beyond)
    {
        let near_clear : AerialSample = aerial_sample(clear_light, transmittance, sampler, uv,
                                                      TERRAIN_RANGE_M * 0.001, max_distance_km);
        // 五级网格之外按最后有覆盖的区间延续可见度，封闭洞穴不会重新得到无条件的远场晴空散射。
        radiance += max(clear.radiance - near_clear.radiance, 0) * saturate(boundary.w);
    }
    return color::rec2020_to_bt709(radiance +
                                   clear.transmittance * color::bt709_to_rec2020(background));
}
}
