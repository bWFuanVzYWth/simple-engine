/// 规范 guide 写入；资源绑定由各入口声明。
module guide_output;
__exported import pt.reconstruction;
__exported import pt.scene;
__exported import pt.key_guides;
import camera.pinhole;
import denoise.guide;
import pt.optical_history;

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

/// 在两路虚拟表面都已投影后，可用跨帧稳定的几何记录修正运动。
public interface IGuideHistory : IOpticalPathKey {
    static func apply(primary: GuideEndpoint, reflected: GuideEndpoint, frame: GuideFrame,
                      pixel: uint2, film: float2, inout motion: float2,
                      inout specular_motion: float2)
        ->void;
};
public struct ProjectedGuideHistory : IGuideHistory {
    public static func boundary(key: uint, hit: uint4, front: bool, reflection: bool, eta: float)
        ->uint {
        return 0;
    }
    public static func apply(primary: GuideEndpoint, reflected: GuideEndpoint, frame: GuideFrame,
                             pixel: uint2, film: float2, inout motion: float2,
                             inout specular_motion: float2)
        ->void {
    }
};

/// 返回两路共用的天空投影，包括旧相机不可见的标记，供应用判定历史可用性。
public func write_sky_guide(images: GuideImages, p: GuideFrame, pixel: uint2, film: float2,
                            direction: float3, flags: uint)
    ->GuideProjection {
    var images = images;
    var flags = flags;
    var motion : float2 = 0;
    if (p.history &&
        (!denoise::denoise_motion(p.previous, film, direction, true, p.extent, motion) ||
         any(abs(motion) > 65504))) {
        motion = 0;
        flags |= pt::GUIDE_MOTION_PROXY | pt::GUIDE_UNPROJECTABLE;
    }
    images.depth[pixel] = denoise::DENOISE_SKY_DEPTH;
    images.motion[pixel] = motion;
    images.normal[pixel] = float4(0, 0, 0, 1);
    images.diffuse[pixel] = float4(0.5, 0.5, 0.5, float(flags | pt::GUIDE_SKY) * (1.0 / 255.0));
    images.specular[pixel] = float4(0.5, 0.5, 0.5, 0);
    images.distance[pixel] = 0;
    images.specular_motion[pixel] = motion;
    return { denoise::DENOISE_SKY_DEPTH, float3(0), motion, flags | GUIDE_SKY };
}

func write_projected_pair<History : IGuideHistory>(images: GuideImages, p: GuideFrame, pixel: uint2,
                                                   film: float2, primary: GuideEndpoint,
                                                   albedos: denoise::DenoiseGuides,
                                                   reflection: GuideEndpoint)
    ->void {
    let surface = project_guide(primary, p.current, p.previous, film, p.extent, p.history);
    let reflected = project_guide(reflection, p.current, p.previous, film, p.extent, p.history);
    let sky = (primary.flags & GUIDE_SKY) != 0;
    let specular = sky ? (primary.use_interface_specular ? primary.interface_specular : float3(0.5))
                       : albedos.specular_albedo;
    images.depth[pixel] = surface.depth;
    images.normal[pixel] =
        sky ? float4(0, 0, 0, 1) : float4(surface.normal, albedos.normal_roughness.w);
    images.diffuse[pixel] = float4(sky ? float3(0.5) : albedos.diffuse_albedo,
                                   float(surface.flags | GUIDE_FOREGROUND) / 255.0);
    images.specular[pixel] = float4(specular, 0);
    images.distance[pixel] = (reflection.flags & GUIDE_SKY) != 0 ? 0 : reflection.distance;
    var motion = surface.motion;
    var specular_motion = (reflected.flags & GUIDE_INVALID) != 0 ? motion : reflected.motion;
    var primary = primary;
    var reflection = reflection;
    primary.flags = surface.flags;
    reflection.flags = reflected.flags;
    History.apply(primary, reflection, p, pixel, film, motion, specular_motion);
    images.motion[pixel] = motion;
    images.specular_motion[pixel] = specular_motion;
}

/// 规范后缀与照明共享实际首命中；理想反射导引只表示该波瓣的代表运动。
public func write_surface_guides<Scene, History : IGuideHistory = ProjectedGuideHistory>(
    scene: Scene, images: GuideImages, p: GuideFrame, pixel: uint2, film: float2,
    first: TransportCall)
    ->void
    where Scene : IGuideScene {
    let primary = trace_guide<Scene, History>(scene, first, p.max_depth);
    var albedos : denoise::DenoiseGuides = {};
    if ((primary.flags & GUIDE_SKY) == 0)
        albedos = guide_albedos(scene.energy(), primary);
    let reflection = trace_reflection_guide<Scene, History>(scene, first, p.max_depth);
    write_projected_pair<History>(images, p, pixel, film, primary, albedos, reflection);
}

/// 材质特化的首命中及后续求交均提供稳定身份；不重复查询表面或计算材质。
public func write_tracked_key_guides<Scene : IIdentifiedGuideTrace, History : IGuideHistory>(
    scene: Scene, images: GuideImages, p: GuideFrame, pixel: uint2, film: float2,
    first: KeyGuideVertex, identity: uint4, camera_origin: float3, camera_direction: float3,
    initial_medium: float4, delta_only: bool = false)
    ->void {
    let primary =
        trace_key_path<Scene, History>(scene, first, identity, camera_origin, camera_direction,
                                       initial_medium, p.max_depth, false, delta_only);
    let reflection =
        trace_key_path<Scene, History>(scene, first, identity, camera_origin, camera_direction,
                                       initial_medium, p.max_depth, true, delta_only);
    write_projected_pair<History>(images, p, pixel, film, primary.endpoint, primary.albedos,
                                  reflection.endpoint);
}

/// 只使用虚拟平面投影的材质键导引，无跨帧光路表。
public func write_key_guides<Scene : IKeyGuideTrace>(scene: Scene, images: GuideImages,
                                                     p: GuideFrame, pixel: uint2, film: float2,
                                                     first: KeyGuideVertex, camera_origin: float3,
                                                     camera_direction: float3,
                                                     initial_medium: float4,
                                                     delta_only: bool = false)
    ->void {
    let trace : UntrackedGuideTrace<Scene> = { scene };
    write_tracked_key_guides<UntrackedGuideTrace<Scene>, ProjectedGuideHistory>(
        trace, images, p, pixel, film, first, 0, camera_origin, camera_direction, initial_medium,
        delta_only);
}

/// SDK 的非负 FP16 HDR 输入；非有限分量归零，负估计只在此边界裁剪。
public func reconstruction_input(hdr: float3)->float4 {
    return float4(select(isfinite(hdr), clamp(hdr, 0, 65504), float3(0)), 1);
}

}
