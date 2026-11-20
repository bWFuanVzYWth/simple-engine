/// 跟随同一 SDK 的输入编码、SH 重建与阴影平方约定，工作色域为场景线性 RGB。
module nrd;
#include "NRD.hlsli"
import denoise.nrd_signal;
[[vk::binding(0, 0)]]
var constants : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var signals : StructuredBuffer<SignalLane>;
[[vk::binding(2, 0)]]
var depths : Texture2D<float>;
[[vk::binding(3, 0)]]
var normals : Texture2D<float4>;
[[vk::binding(4, 0)]]
var albedo : Texture2D<float4>;
[[vk::binding(5, 0)]]
var specular_color : Texture2D<float4>;
[[vk::binding(6, 0)]]
[[vk::image_format("rgb10_a2")]]
var packed_normal : RWTexture2D<float4>;
[[vk::binding(7, 0)]]
[[vk::image_format("rgba16f")]]
var diffuse0 : RWTexture2D<float4>;
[[vk::binding(8, 0)]]
[[vk::image_format("rgba16f")]]
var diffuse1 : RWTexture2D<float4>;
[[vk::binding(9, 0)]]
[[vk::image_format("rgba16f")]]
var specular0 : RWTexture2D<float4>;
[[vk::binding(10, 0)]]
[[vk::image_format("rgba16f")]]
var specular1 : RWTexture2D<float4>;
[[vk::binding(11, 0)]]
[[vk::image_format("r32f")]]
var penumbra : RWTexture2D<float>;
[[vk::binding(12, 0)]]
[[vk::image_format("r32f")]]
var disocclusion : RWTexture2D<float>;
[[vk::binding(13, 0)]]
var shadow : Texture2D<float>;
[[vk::binding(14, 0)]]
var surface_radiance : RWStructuredBuffer<float4>;

[[vk::binding(15, 0)]]
var spec_depth : Texture2D<float>;
[[vk::binding(16, 0)]]
var spec_normal : Texture2D<float2>;
[[vk::binding(17, 0)]]
var spec_motion : Texture2D<float4>;
[[vk::binding(18, 0)]]
[[vk::image_format("rgb10_a2")]]
var spec_packed_normal : RWTexture2D<float4>;
[[vk::binding(19, 0)]]
[[vk::image_format("r32f")]]
var spec_disocclusion : RWTexture2D<float>;
[[vk::binding(20, 0)]]
var diff_motion : Texture2D<float4>;

/// 带色界面和吸收均已折入对应导引的方向能量。相同下限用于编码和解码。
func modulation(energy: float3)->float3 {
    return max(energy, NRD_MATERIAL_FACTOR_MIN_SCALE);
}
func norm_hit(distance: float, z: float, roughness: float)->float {
    return distance > 0
               ? max(REBLUR_FrontEnd_GetNormHitDist(distance, z, float3(3, .1, 20), roughness),
                     0.00006103515625)
               : 0;
}
func pack_lobe(uint2 colors0, uint2 colors1, uint2 dir0, uint2 dir1, float3 factor, float z,
               float roughness, out float4 sh0, out float4 sh1)
    ->void {
    let a = unpack_signal(colors0);
    let b = unpack_signal(colors1);
    var first0 : float4;
    var first1 : float4;
    let ca = REBLUR_FrontEnd_PackSh(a.xyz / factor, norm_hit(a.w, z, roughness),
                                    unpack_signal(dir0).xyz, first0, true);
    let cb = REBLUR_FrontEnd_PackSh(b.xyz / factor, norm_hit(b.w, z, roughness),
                                    unpack_signal(dir1).xyz, first1, true);
    sh0 = (ca + cb) * .5;
    // 无采样的 hit distance 为 0，不能与有效距离一起取均值。
    if (ca.w == 0 || cb.w == 0)
        sh0.w = max(ca.w, cb.w);
    sh1 = (first0 + first1) * .5;
}
[shader("compute")]
[numthreads(8, 8, 1)]
func pack(id: uint3: SV_DispatchThreadID)->void {
    let c = constants[0];
    if (any(id.xy >= c.xy))
        return;
    let pos = int3(id.xy, 0);
    let n = normals.Load(pos);
    let spec = specular_color.Load(pos);
    let base = albedo.Load(pos);
    let interior = uint(diff_motion.Load(pos).w);
    let material = float(interior & 3u);
    let reflected = uint(spec_motion.Load(pos).w);
    packed_normal[id.xy] = NRD_FrontEnd_PackNormalAndRoughness(n.xyz, n.w, material);
    // 高光 G-buffer 已给出该分量的时序锚点。指定 SDK 的表面运动模式，避免再次外推反射位置。
    spec_packed_normal[id.xy] =
        NRD_FrontEnd_PackNormalAndRoughness(from_oct_normal(spec_normal.Load(pos)), spec.w, 3.0);
    disocclusion[id.xy] = float((interior & 8u) != 0 || material == 2);
    spec_disocclusion[id.xy] = float((reflected & 8u) != 0 || (reflected & 3u) == 2u);
    if (base.w == 0) {
        diffuse0[id.xy] = 0;
        diffuse1[id.xy] = 0;
        specular0[id.xy] = 0;
        specular1[id.xy] = 0;
        penumbra[id.xy] = 65504;
        return;
    }
    let index = id.y * c.x + id.x;
    let a = signals[index * 2];
    let b = signals[index * 2 + 1];
    let df = modulation(base.xyz);
    let sf = modulation(spec.xyz);
    let z = depths.Load(pos);
    var sh0 : float4;
    var sh1 : float4;
    pack_lobe(a.diffuse, b.diffuse, a.diffuse_direction, b.diffuse_direction, df, z, 1, sh0, sh1);
    diffuse0[id.xy] = sh0;
    diffuse1[id.xy] = sh1;
    pack_lobe(a.specular, b.specular, a.specular_direction, b.specular_direction, sf,
              spec_depth.Load(pos), spec.w, sh0, sh1);
    specular0[id.xy] = sh0;
    specular1[id.xy] = sh1;
    penumbra[id.xy] = SIGMA_FrontEnd_PackPenumbra(unpack_signal(a.sun).w, asfloat(c.z));
}
[shader("compute")]
[numthreads(8, 8, 1)]
func resolve(id: uint3: SV_DispatchThreadID)->void {
    let c = constants[0];
    if (any(id.xy >= c.xy))
        return;
    let pos = int3(id.xy, 0);
    let base = albedo.Load(pos);
    let index = id.y * c.x + id.x;
    let raw = surface_radiance[index];
    if (raw.w < 0) {
        surface_radiance[index] = float4(0, 0, 0, raw.w);
        return;
    }
    if (base.w == 0)
        return;
    let a = signals[index * 2];
    let b = signals[index * 2 + 1];
    let n = normals.Load(pos);
    let view_angle = unpack_signal(a.view_angle);
    let v = normalize(view_angle.xyz);
    let reflected = specular_color.Load(pos);
    let spec_n = from_oct_normal(spec_normal.Load(pos));
    let diff = REBLUR_BackEnd_UnpackSh(diffuse0[id.xy], diffuse1[id.xy].xyz);
    let spec = REBLUR_BackEnd_UnpackSh(specular0[id.xy], specular1[id.xy].xyz);
    let d = (base.w > 1 ? NRD_SG_ExtractColor(diff) : NRD_SG_ResolveDiffuse(diff, n.xyz, v, n.w)) *
            modulation(base.xyz);
    let r = ((uint(spec_motion.Load(pos).w) & 4u) != 0
                 ? NRD_SG_ExtractColor(spec)
                 : NRD_SG_ResolveSpecular(spec, spec_n, v, reflected.w)) *
            modulation(reflected.xyz);
    let direct = (unpack_signal(a.sun).xyz + unpack_signal(b.sun).xyz) * .5;
    if (!any(direct > 0)) {
        surface_radiance[index] = float4(max(d + r, 0), raw.w);
        return;
    }
    // 在双方均被遮挡的接触处保留亚像素硬影；半影超过一个像素时完全使用 SIGMA。
    let distance = max(unpack_signal(a.sun).w, unpack_signal(b.sun).w);
    let radius = SIGMA_FrontEnd_PackPenumbra(distance, asfloat(c.z));
    let cos_light = min(unpack_signal(a.diffuse_direction).w, unpack_signal(b.diffuse_direction).w);
    let footprint = max(abs(depths.Load(pos)) * view_angle.w * cos_light, 1e-9);
    let contact_weight = distance >= NRD_FP16_MAX ? 1 : smoothstep(0.25, 1, radius / footprint);
    let visibility = contact_weight * SIGMA_BackEnd_UnpackShadow(shadow.Load(pos));
    let surface = direct * visibility + d + r;
    surface_radiance[index] = float4(max(surface, 0), raw.w);
}
