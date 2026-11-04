/// 通用降噪辅助量；资源格式、历史接受与重建 SDK 由应用适配器选择。
module denoise;

__exported import bsdf.openpbr.denoise;
import pt.scene;
__exported import geometry.camera;
import "bsdf/openpbr/math/microfacet_energy.slang";

namespace pt
{
/// 未命中线性深度；不表示有限几何或设备深度。
public static const float DENOISE_SKY_DEPTH = 3.402823466e38f;

/// 正交单位相机基；position.w=tan(verticalFov/2)，forward.w=aspect，right/up.w 保留。
/// 前后帧的 position 必须处于同一场景锚点坐标系。
public typedef geometry::Camera DenoiseCamera;

/// 当前首个实际表面的世界法线、线性 roughness 和方向分量能量。
/// 法线朝向观察者；没有光学 PSR、入射照明、路径吞吐或曝光。
public struct DenoiseGuides
{
    public float4 normal_roughness;
    public float3 diffuse_albedo;
    public float3 specular_albedo;
};

/// 从实际首命中翻译结果生成材质辅助量；只读，不改变积分器或随机消费。
public DenoiseGuides denoise_surface<Energy>(Energy energy, SurfaceMaterial material, float3 normal,
                                             float3 view, float4 incident)
    where Energy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    openpbr::DenoiseAlbedos albedos = openpbr::denoise_albedos(energy, material.pbr, normal, view,
                                                               incident, material.transmitted);
    DenoiseGuides result;
    result.normal_roughness =
        float4(dot(normal, view) < 0 ? -normal : normal, material.pbr.base_roughness.w);
    result.diffuse_albedo = albedos.diffuse;
    result.specular_albedo = albedos.specular;
    return result;
}

/// 前向线性深度，单位同场景；不是射线距离。调用者负责拒绝 <=0 或非有限值。
public float denoise_depth(DenoiseCamera camera, float3 position)
{
    return geometry::camera_depth(camera, position);
}

/// sample_uv 使用左上原点且含当前帧像素 jitter；返回单位世界方向。
public float3 denoise_direction(DenoiseCamera camera, float2 sample_uv)
{
    return geometry::camera_direction(camera, sample_uv);
}

/// previous-current 的输入像素运动，左上原点；前帧相机不含 jitter。
/// target 为对应前帧世界位置，directional=true 时为世界方向（天空无平移视差）。
/// 当前 sample_uv 必须含实际追踪 jitter，静止场景运动才不混入 jitter；不添加前帧 jitter。
/// 投影失败返回 false 和有限零。对象对应关系、历史重置与输出格式由宿主维护。
public bool denoise_motion(DenoiseCamera previous, float2 sample_uv, float3 target,
                           bool directional, uint2 extent, out float2 pixels)
{
    pixels = 0;
    float3 relative = directional ? target : target - previous.position.xyz;
    geometry::CameraProjection projected =
        geometry::camera_project_direction(previous, relative, extent);
    if (!projected.valid)
        return false;
    float2 result = projected.pixel - sample_uv * float2(extent);
    if (any(!isfinite(result)))
        return false;
    pixels = result;
    return true;
}
}
