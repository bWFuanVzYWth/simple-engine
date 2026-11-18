/// 场景命中解析得到的表面材质与介质边界。
module surface_material;
import openpbr.contract.surface;

namespace transport {
/// 当前顶点的局部翻译结果；transmitted 是边界另一侧实际 IOR／RGB extinction。
/// emission 已处理发光侧，boundary_filter 仅在 delta solid 穿越界面时相乘。
public struct SurfaceMaterial {
    public var pbr : openpbr::Material;
    public var transmitted : float4;
    public var emission : float3;
    public var boundary_filter : float3;
    /// 单位着色法线。零向量沿用命中的几何法线，几何偏移和介质跨界也使用几何法线。
    public var normal : float3;
};
}
