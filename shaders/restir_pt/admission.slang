/// 复用候选的共同准入；只裁剪确定性的屏幕/几何域，不代替双向shift的BSDF支持检查。
module admission;
import geometry.ray;

namespace restir_pt
{
/// 投影位置含当前采样jitter；donor地址扣当前值，前帧jitter只用于路径重放。
/// 历史有效性与稳定身份由所有者/场景另行校验，不要求同primitive、材质或法线。
public bool temporal_candidate(float2 projectedPixel, bool projectedValid, uint2 extent,
                               float2 currentJitter, out uint2 donor)
{
    donor = 0;
    float2 location = projectedPixel - currentJitter;
    if (!projectedValid || any(!isfinite(location)) || any(location < 0.0) ||
        any(location >= float2(extent)))
        return false;
    donor = uint2(location);
    return true;
}

/// 仅准入阶段存活的几何摘要；bothSides表示双面边界或可能有薄层反侧散射。
/// 保守声明只扩大候选域；最终有效支持仍由两次精确shift求值决定。
public struct AdmissionSurface
{
    public float3 normal;
    public float distance;
    public bool bothSides;
};

/// 将几何法线朝向共同相机；不读取BSDF/纹理，不把双面性改写为路径透射事件。
public AdmissionSurface admission_surface(geometry::SurfacePoint point, float3 camera,
                                          bool bothSides)
{
    float3 view = camera - point.position;
    return { dot(point.normal, view) >= 0.0 ? point.normal : -point.normal, length(view),
             bothSides };
}

/// 对称paired准入：保留至少60度法线邻域与10%深度邻域。
/// 任一边界可双侧散射时比较无向法线，避免薄壁SSS/双面背面被几何粗筛提前丢弃。
/// 该集合不依赖随机路径、shift成败或辐射；逆向使用同一集合，供pairwise MIS和CV共享。
public bool spatial_candidate(AdmissionSurface canonical, AdmissionSurface neighbor)
{
    float cosine = dot(canonical.normal, neighbor.normal);
    if (canonical.bothSides || neighbor.bothSides)
        cosine = abs(cosine);
    return cosine >= 0.5 && abs(canonical.distance - neighbor.distance) <
                                0.1 * max(canonical.distance, neighbor.distance);
}
}
