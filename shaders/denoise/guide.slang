/// 降噪导引的深度、法线和运动约定。
module guide;
import camera.pinhole;

namespace denoise {
/// 未命中线性深度；不表示有限几何或设备深度。
public static const let DENOISE_SKY_DEPTH = 3.402823466e38f;

/// 当前首个实际表面的世界法线、线性 roughness 和方向分量能量。
/// 法线朝向观察者；没有光学 PSR、入射照明、路径吞吐或曝光。
public struct DenoiseGuides {
    public var normal_roughness : float4;
    public var diffuse_albedo : float3;
    public var specular_albedo : float3;
};

/// previous-current 的输入像素运动，左上原点；前帧相机不含 jitter。
/// target 为对应前帧世界位置，directional=true 时为世界方向（天空无平移视差）。
/// 当前 sample_uv 必须含实际追踪 jitter，静止场景运动才不混入 jitter；不添加前帧 jitter。
/// 投影失败返回 false 和有限零。对象对应关系、历史重置与输出格式由宿主维护。
public func denoise_motion(previous: camera::Camera, sample_uv: float2, target: float3,
                           directional: bool, extent: uint2, out pixels: float2)
    ->bool {
    pixels = 0;
    let relative : float3 = directional ? target : target - previous.position.xyz;
    let projected : camera::CameraProjection =
                        camera::camera_project_direction(previous, relative, extent);
    if (!projected.valid)
        return false;
    let result = projected.pixel - sample_uv * float2(extent);
    if (any(!isfinite(result)))
        return false;
    pixels = result;
    return true;
}
}
