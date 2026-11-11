/// 规范 guide 写入；资源绑定由各入口声明。
module guide_output;
import geometry.camera;
__exported import pt.reconstruction;
import pt.denoise;
__exported import pt.scene;
__exported import pt.key_guides;

namespace pt
{
/// alpha 的内部标记，SDK 的 diffuse/specular 仍只消费 RGB。
public static const uint GUIDE_FOREGROUND = 64u;

public struct GuideImages
{
    public RWTexture2D<float> depth;
    public RWTexture2D<float2> motion;
    public RWTexture2D<float4> normal;
    public RWTexture2D<float4> diffuse;
    public RWTexture2D<float4> specular;
    public RWTexture2D<float> distance;
    public RWTexture2D<float2> specular_motion;
};

/// 已完成坐标适配的相机与帧参数；不依赖应用 uniform 布局。
public struct GuideFrame
{
    public DenoiseCamera current;
    public DenoiseCamera previous;
    public uint2 extent;
    public uint max_depth;
    public bool history;
};

/// escape 的 motion 使用原相机方向；光学链中的 escape 仍由 flags 保留前景和代理事实。
public void write_sky_guide(GuideImages images, GuideFrame p, uint2 pixel, float2 film,
                            float3 direction, uint flags)
{
    float2 motion = 0;
    if (p.history && (!pt::denoise_motion(p.previous, film, direction, true, p.extent, motion) ||
                      any(abs(motion) > 65504)))
    {
        motion = 0;
        flags |= pt::GUIDE_MOTION_PROXY;
    }
    images.depth[pixel] = pt::DENOISE_SKY_DEPTH;
    images.motion[pixel] = motion;
    images.normal[pixel] = float4(0, 0, 0, 1);
    images.diffuse[pixel] = float4(0.5, 0.5, 0.5, float(flags | pt::GUIDE_SKY) / 255);
    images.specular[pixel] = float4(0.5, 0.5, 0.5, 0);
    images.distance[pixel] = 0;
    images.specular_motion[pixel] = motion;
}

/// 规范后缀与照明状态独立；两者只共享已经 resolve_hit 的实际首命中。
/// water/glass/TIR 使用真实完整材料。粗糙首面反射运动使用独立理想反射代理，
/// 不声称等于随机粗反射光流；每条 guide 最多另查 N-1 次，不改写照明。
public void write_surface_guides<Scene>(Scene scene, GuideImages images, GuideFrame p, uint2 pixel,
                                        float2 film, pt::TransportCall first)
    where Scene : pt::IGuideScene
{
    pt::GuideEndpoint endpoint = pt::trace_guide(scene, first, p.max_depth);
    if ((endpoint.flags & pt::GUIDE_SKY) != 0)
    {
        write_sky_guide(images, p, pixel, film, first.path.direction,
                        endpoint.flags | GUIDE_FOREGROUND);
        if (endpoint.use_interface_specular)
            images.specular[pixel] = float4(endpoint.interface_specular, 0);
    }
    else
    {
        pt::GuideProjection projection =
            pt::project_guide(endpoint, p.current, p.previous, film, p.extent, p.history);
        pt::DenoiseGuides albedos = pt::guide_albedos(scene.energy(), endpoint);
        images.depth[pixel] = projection.depth;
        images.motion[pixel] = projection.motion;
        images.normal[pixel] = float4(projection.normal, albedos.normal_roughness.w);
        images.diffuse[pixel] =
            float4(albedos.diffuse_albedo, float(projection.flags | GUIDE_FOREGROUND) / 255);
        images.specular[pixel] = float4(albedos.specular_albedo, 0);
    }
    pt::GuideEndpoint reflection = pt::trace_reflection_guide(scene, first, p.max_depth);
    pt::GuideProjection reflected =
        pt::project_guide(reflection, p.current, p.previous, film, p.extent, p.history);
    images.distance[pixel] = (reflection.flags & pt::GUIDE_SKY) != 0 ? 0 : reflection.distance;
    images.specular_motion[pixel] =
        (reflected.flags & pt::GUIDE_INVALID) != 0 ? images.motion[pixel] : reflected.motion;
}
/// 生产HitObject/known-surface路径只消费按材质特化后返回的数值，不在RG求BSDF。
public void write_key_guides<Scene : pt::IKeyGuideTrace>(
    Scene scene, GuideImages images, GuideFrame p, uint2 pixel, float2 film,
    pt::KeyGuideVertex first, float3 cameraOrigin, float3 cameraDirection, float4 initialMedium,
    bool deltaOnly = false)
{
    pt::KeyGuideEndpoint keyed = pt::trace_key_guide(scene, first, cameraOrigin, cameraDirection,
                                                     initialMedium, p.max_depth, false, deltaOnly);
    pt::GuideEndpoint endpoint = keyed.endpoint;
    if ((endpoint.flags & pt::GUIDE_SKY) != 0)
    {
        write_sky_guide(images, p, pixel, film, cameraDirection, endpoint.flags | GUIDE_FOREGROUND);
        if (endpoint.use_interface_specular)
            images.specular[pixel] = float4(endpoint.interface_specular, 0);
    }
    else
    {
        pt::GuideProjection projection =
            pt::project_guide(endpoint, p.current, p.previous, film, p.extent, p.history);
        images.depth[pixel] = projection.depth;
        images.motion[pixel] = projection.motion;
        images.normal[pixel] = float4(projection.normal, keyed.albedos.normal_roughness.w);
        images.diffuse[pixel] =
            float4(keyed.albedos.diffuse_albedo, float(projection.flags | GUIDE_FOREGROUND) / 255);
        images.specular[pixel] = float4(keyed.albedos.specular_albedo, 0);
    }
    pt::KeyGuideEndpoint reflectedKey = pt::trace_key_guide(
        scene, first, cameraOrigin, cameraDirection, initialMedium, p.max_depth, true, deltaOnly);
    pt::GuideEndpoint reflection = reflectedKey.endpoint;
    pt::GuideProjection reflected =
        pt::project_guide(reflection, p.current, p.previous, film, p.extent, p.history);
    images.distance[pixel] = (reflection.flags & pt::GUIDE_SKY) != 0 ? 0 : reflection.distance;
    images.specular_motion[pixel] =
        (reflected.flags & pt::GUIDE_INVALID) != 0 ? images.motion[pixel] : reflected.motion;
}

/// SDK 的非负 FP16 HDR 输入；非有限分量归零，负估计只在此边界裁剪。
public float4 reconstruction_input(float3 hdr)
{
    return float4(select(isfinite(hdr), clamp(hdr, 0, 65504), float3(0)), 1);
}

}
