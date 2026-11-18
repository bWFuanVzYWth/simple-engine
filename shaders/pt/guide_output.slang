/// 规范 guide 写入；资源绑定由各入口声明。
module guide_output;
__exported import pt.reconstruction;
__exported import pt.scene;
__exported import pt.key_guides;
import camera.pinhole;
import denoise.guide;

namespace pt {
/// alpha 的内部标记，SDK 的 diffuse/specular 仍只消费 RGB。
public static const let GUIDE_FOREGROUND = 64u;

public struct GuideImages {
    public var depth : RWTexture2D<float>;
    public var motion : RWTexture2D<float2>;
    public var normal : RWTexture2D<float4>;
    public var diffuse : RWTexture2D<float4>;
    public var specular : RWTexture2D<float4>;
    public var distance : RWTexture2D<float>;
    public var specular_motion : RWTexture2D<float2>;
};

/// 已完成坐标适配的相机与帧参数；不依赖应用 uniform 布局。
public struct GuideFrame {
    public var current : camera::Camera;
    public var previous : camera::Camera;
    public var extent : uint2;
    public var max_depth : uint;
    public var history : bool;
};

/// escape 的 motion 使用原相机方向；光学链中的 escape 仍由 flags 保留前景和代理事实。
public func write_sky_guide(images: GuideImages, p: GuideFrame, pixel: uint2, film: float2,
                            direction: float3, flags: uint)
    ->void {
    var images = images;
    var flags = flags;
    var motion : float2 = 0;
    if (p.history &&
        (!denoise::denoise_motion(p.previous, film, direction, true, p.extent, motion) ||
         any(abs(motion) > 65504))) {
        motion = 0;
        flags |= pt::GUIDE_MOTION_PROXY;
    }
    images.depth[pixel] = denoise::DENOISE_SKY_DEPTH;
    images.motion[pixel] = motion;
    images.normal[pixel] = float4(0, 0, 0, 1);
    images.diffuse[pixel] = float4(0.5, 0.5, 0.5, float(flags | pt::GUIDE_SKY) * (1.0 / 255.0));
    images.specular[pixel] = float4(0.5, 0.5, 0.5, 0);
    images.distance[pixel] = 0;
    images.specular_motion[pixel] = motion;
}

/// 规范后缀与照明状态独立；两者只共享已经 resolve_hit 的实际首命中。
/// water/glass/TIR 使用真实完整材料。粗糙首面反射运动使用独立理想反射代理，
/// 不声称等于随机粗反射光流；每条 guide 最多另查 N-1 次，不改写照明。
public func write_surface_guides<Scene>(scene: Scene, images: GuideImages, p: GuideFrame,
                                        pixel: uint2, film: float2, first: pt::TransportCall)
    ->void
    where Scene : pt::IGuideScene {
    var images = images;
    let endpoint : pt::GuideEndpoint = pt::trace_guide(scene, first, p.max_depth);
    if ((endpoint.flags & pt::GUIDE_SKY) != 0) {
        write_sky_guide(images, p, pixel, film, first.path.direction,
                        endpoint.flags | GUIDE_FOREGROUND);
        if (endpoint.use_interface_specular)
            images.specular[pixel] = float4(endpoint.interface_specular, 0);
    } else {
        let projection : pt::GuideProjection = pt::project_guide(endpoint, p.current, p.previous,
                                                                 film, p.extent, p.history);
        let albedos : denoise::DenoiseGuides = pt::guide_albedos(scene.energy(), endpoint);
        images.depth[pixel] = projection.depth;
        images.motion[pixel] = projection.motion;
        images.normal[pixel] = float4(projection.normal, albedos.normal_roughness.w);
        images.diffuse[pixel] = float4(albedos.diffuse_albedo,
                                       float(projection.flags | GUIDE_FOREGROUND) * (1.0 / 255.0));
        images.specular[pixel] = float4(albedos.specular_albedo, 0);
    }
    let reflection : pt::GuideEndpoint = pt::trace_reflection_guide(scene, first, p.max_depth);
    let reflected : pt::GuideProjection = pt::project_guide(reflection, p.current, p.previous, film,
                                                            p.extent, p.history);
    images.distance[pixel] = (reflection.flags & pt::GUIDE_SKY) != 0 ? 0 : reflection.distance;
    images.specular_motion[pixel] =
        (reflected.flags & pt::GUIDE_INVALID) != 0 ? images.motion[pixel] : reflected.motion;
}
/// 生产HitObject/known-surface路径只消费按材质特化后返回的数值，不在RG求BSDF。
public func write_key_guides<Scene : pt::IKeyGuideTrace>(
    scene: Scene, images: GuideImages, p: GuideFrame, pixel: uint2, film: float2,
    first: pt::KeyGuideVertex, camera_origin: float3, camera_direction: float3,
    initial_medium: float4, delta_only: bool = false)
    ->void {
    var images = images;
    let keyed : pt::KeyGuideEndpoint =
                    pt::trace_key_guide(scene, first, camera_origin, camera_direction,
                                        initial_medium, p.max_depth, false, delta_only);
    let endpoint : pt::GuideEndpoint = keyed.endpoint;
    if ((endpoint.flags & pt::GUIDE_SKY) != 0) {
        write_sky_guide(images, p, pixel, film, camera_direction,
                        endpoint.flags | GUIDE_FOREGROUND);
        if (endpoint.use_interface_specular)
            images.specular[pixel] = float4(endpoint.interface_specular, 0);
    } else {
        let projection : pt::GuideProjection = pt::project_guide(endpoint, p.current, p.previous,
                                                                 film, p.extent, p.history);
        images.depth[pixel] = projection.depth;
        images.motion[pixel] = projection.motion;
        images.normal[pixel] = float4(projection.normal, keyed.albedos.normal_roughness.w);
        images.diffuse[pixel] = float4(keyed.albedos.diffuse_albedo,
                                       float(projection.flags | GUIDE_FOREGROUND) * (1.0 / 255.0));
        images.specular[pixel] = float4(keyed.albedos.specular_albedo, 0);
    }
    let reflected_key : pt::KeyGuideEndpoint =
                            pt::trace_key_guide(scene, first, camera_origin, camera_direction,
                                                initial_medium, p.max_depth, true, delta_only);
    let reflection : pt::GuideEndpoint = reflected_key.endpoint;
    let reflected : pt::GuideProjection = pt::project_guide(reflection, p.current, p.previous, film,
                                                            p.extent, p.history);
    images.distance[pixel] = (reflection.flags & pt::GUIDE_SKY) != 0 ? 0 : reflection.distance;
    images.specular_motion[pixel] =
        (reflected.flags & pt::GUIDE_INVALID) != 0 ? images.motion[pixel] : reflected.motion;
}

/// SDK 的非负 FP16 HDR 输入；非有限分量归零，负估计只在此边界裁剪。
public func reconstruction_input(hdr: float3)->float4 {
    return float4(select(isfinite(hdr), clamp(hdr, 0, 65504), float3(0)), 1);
}

}
