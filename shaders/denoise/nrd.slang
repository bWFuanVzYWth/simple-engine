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
var f0 : Texture2D<float4>;
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

func pack_lobe(uint2 colors0, uint2 colors1, uint2 dir0, uint2 dir1, float3 factor, float z,
               float roughness, out float4 sh0, out float4 sh1)
    ->void {
    let a = unpack_signal(colors0);
    let b = unpack_signal(colors1);
    var first0 : float4;
    var first1 : float4;
    let params = float3(3, .1, 20);
    let ca = REBLUR_FrontEnd_PackSh(a.xyz / factor,
                                    REBLUR_FrontEnd_GetNormHitDist(a.w, z, params, roughness),
                                    unpack_signal(dir0).xyz, first0, true);
    let cb = REBLUR_FrontEnd_PackSh(b.xyz / factor,
                                    REBLUR_FrontEnd_GetNormHitDist(b.w, z, params, roughness),
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
    let spec = f0.Load(pos);
    let base = albedo.Load(pos);
    packed_normal[id.xy] = NRD_FrontEnd_PackNormalAndRoughness(n.xyz, n.w, spec.w);
    disocclusion[id.xy] = float(spec.w == 2);
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
    var df : float3;
    var sf : float3;
    NRD_MaterialFactors(n.xyz, normalize(unpack_signal(a.fraction).xyz), base.xyz, spec.xyz, n.w,
                        df, sf);
    let z = depths.Load(pos);
    var sh0 : float4;
    var sh1 : float4;
    pack_lobe(a.diffuse, b.diffuse, a.diffuse_direction, b.diffuse_direction, df, z, 1, sh0, sh1);
    diffuse0[id.xy] = sh0;
    diffuse1[id.xy] = sh1;
    pack_lobe(a.specular, b.specular, a.specular_direction, b.specular_direction, sf, z, n.w, sh0,
              sh1);
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
    if (base.w == 0 || raw.w < 0)
        return;
    let a = signals[index * 2];
    let b = signals[index * 2 + 1];
    let n = normals.Load(pos);
    let v = normalize(unpack_signal(a.fraction).xyz);
    var df : float3;
    var sf : float3;
    NRD_MaterialFactors(n.xyz, v, base.xyz, f0.Load(pos).xyz, n.w, df, sf);
    let diff = REBLUR_BackEnd_UnpackSh(diffuse0[id.xy], diffuse1[id.xy].xyz);
    let spec = REBLUR_BackEnd_UnpackSh(specular0[id.xy], specular1[id.xy].xyz);
    let direct = (unpack_signal(a.sun).xyz + unpack_signal(b.sun).xyz) * .5;
    let surface = unpack_signal(a.emission).xyz +
                  direct * SIGMA_BackEnd_UnpackShadow(shadow.Load(pos)) +
                  NRD_SG_ResolveDiffuse(diff, n.xyz, v, n.w) * df +
                  NRD_SG_ResolveSpecular(spec, n.xyz, v, n.w) * sf;
    surface_radiance[index] = float4(max(surface, 0), raw.w);
}
