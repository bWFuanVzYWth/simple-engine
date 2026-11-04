/// 正交 pinhole 相机；左上像素原点、正 view-Z，输出 aspect 与输入采样尺寸相互独立。
/// 相机由 CPU 校验并归一化，所有点和前后相机须处于同一显式锚点。
/// 射线计算及缩放基长度须处于 f32 有效范围；投影另以 valid 显式报告数值失败。
module camera;

namespace geometry
{
/// 与 camera::Relative::words 一致的四个 float4；不含帧 jitter。
public struct Camera
{
    /// xyz 为锚点相对位置，w 为 tan(verticalFov/2)。
    public float4 position;
    /// xyz 为单位前向，w 为最终输出 aspect。
    public float4 forward;
    /// xyz 为单位右向，w 保留为零。
    public float4 right;
    /// xyz 为单位上向，w 保留为零。
    public float4 up;
};

/// 无 jitter 投影；非法结果保留有限零，valid 与画面内外无关。
public struct CameraProjection
{
    /// 左上原点的实际输入像素位置；可在画面外。
    public float2 pixel;
    /// 正向线性深度，不是射线距离。
    public float view_z;
    /// 深度、输入与投影均合法。
    public bool valid;
};

/// 从追踪用的缩放半视锥基构造四个 float4；输入须已由 CPU 校验为非零正交有限基。
/// 原长度 f/r/u 转为 tangent=u/f、aspect=r/u，position 不包含其它应用参数。
public Camera camera_from_scaled(float3 position, float3 forward, float3 right, float3 up)
{
    float3 lengths = float3(length(forward), length(right), length(up));
    Camera result;
    result.position = float4(position, lengths.z / lengths.x);
    result.forward = float4(forward / lengths.x, lengths.y / lengths.z);
    result.right = float4(right / lengths.y, 0);
    result.up = float4(up / lengths.z, 0);
    return result;
}

/// 非零输入尺寸中的像素中心加 jitter；jitter 为输入像素单位，须在 [-.5,.5]。
public float2 camera_sample_uv(uint2 pixel, uint2 extent, float2 jitter)
{
    return (float2(pixel) + 0.5 + jitter) / float2(extent);
}

/// 从已包含 jitter 的左上原点 UV 生成单位世界方向；UV 须有限。
public float3 camera_direction(Camera camera, float2 sample_uv)
{
    return camera_scaled_direction(camera.forward.xyz,
                                   camera.right.xyz * (camera.forward.w * camera.position.w),
                                   camera.up.xyz * camera.position.w, sample_uv);
}

/// 已校验的缩放半视锥基直接生成射线，不重新归一化三个基；适合主追踪已有的参数编码。
public float3 camera_scaled_direction(float3 forward, float3 right, float3 up, float2 sample_uv)
{
    float2 screen = sample_uv * 2.0 - 1.0;
    return normalize(forward + right * screen.x - up * screen.y);
}

/// 输入像素中心加 jitter 的单位方向；起点为 camera.position.xyz。
public float3 camera_ray(Camera camera, uint2 pixel, uint2 extent, float2 jitter)
{
    return camera_direction(camera, camera_sample_uv(pixel, extent, jitter));
}

/// 前向线性深度；position 须与相机处于同一锚点。
public float camera_depth(Camera camera, float3 position)
{
    return dot(position - camera.position.xyz, camera.forward.xyz);
}

/// 投影世界方向，不含相机平移或帧 jitter；extent 为非零输入尺寸。
public CameraProjection camera_project_direction(Camera camera, float3 direction, uint2 extent)
{
    CameraProjection result = {};
    float depth = dot(direction, camera.forward.xyz);
    float2 denominator = float2(camera.forward.w, 1) * (depth * camera.position.w);
    if (!(depth > 1e-6) || !isfinite(depth) || any(denominator <= 0) || any(!isfinite(denominator)))
        return result;
    float2 screen =
        float2(dot(direction, camera.right.xyz), -dot(direction, camera.up.xyz)) / denominator;
    float2 pixel = (screen * 0.5 + 0.5) * float2(extent);
    if (any(!isfinite(pixel)))
        return result;
    result.pixel = pixel;
    result.view_z = depth;
    result.valid = true;
    return result;
}

/// 无 jitter 的世界相对点投影；深度 <=1e-6 或数值不可表示返回 valid=false。
public CameraProjection camera_project(Camera camera, float3 position, uint2 extent)
{
    return camera_project_direction(camera, position - camera.position.xyz, extent);
}

float camera_radical(uint index, uint base)
{
    float weight = 1;
    float value = 0;
    while (index != 0)
    {
        weight /= float(base);
        value += float(index % base) * weight;
        index /= base;
    }
    return value - 0.5;
}

/// Halton(2,3) 中心偏移；phases 须由 camera::Jitter 准备且在 8..=256，index 为绝对帧编号。
/// 与 CPU 使用相同的有限周期，返回输入像素单位；SDK 投影 jitter 由适配器取负。
public float2 camera_jitter(uint index, uint phases)
{
    uint phase = index % phases + 1;
    return float2(camera_radical(phase, 2), camera_radical(phase, 3));
}
}
