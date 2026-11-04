/// 编译期场景适配契约。长度统一为场景单位，颜色为线性 BT.709；资源由宿主保活。
module scene;

import geometry.ray;
import bsdf.openpbr.api;

namespace pt
{
public static const uint PATH_ACTIVE = 0u;
public static const uint PATH_DONE = 1u;
public static const uint PATH_INVALID = 2u;
public static const uint PATH_DIELECTRIC_LIMIT = 3u;
public static const float FAR = 1.0e8;

/// 紧凑命中缓存；identity、texel 的编码仅由场景解释，不保存完整 OpenPBR 材料。
public struct Hit
{
    public geometry::SurfacePoint surface;
    public uint4 identity;
    /// 场景的紧凑程序分类，由 prefix／shade 解释，不等同于 OpenPBR Capability。
    public uint capability;
    public float distance;
    public float area;
    public bool front;
    public float4 texel;
};

/// 路径唯一可变状态；depth 为已消费顶点数，previous_pdf=0 表示离散前驱。
/// 非 ACTIVE 输出的 origin、previous、previous_pdf 规范为零，不再携带下一段或 MIS 身份。
public struct PathState
{
    public float3 origin;
    public float3 direction;
    public float3 throughput;
    public float3 radiance;
    public float4 medium;
    public float3 previous;
    public float previous_pdf;
    public float eta_scale;
    public uint depth;
    public uint dielectric_events;
    public uint status;
};

/// 当前顶点的局部翻译结果；transmitted 是边界另一侧实际 IOR／RGB extinction。
/// emission 已处理发光侧，boundary_filter 仅在 delta solid 穿越界面时相乘。
public struct SurfaceMaterial
{
    public openpbr::Material pbr;
    public float4 transmitted;
    public float3 emission;
    public float3 boundary_filter;
    /// 单位着色法线；零向量沿用Hit的几何法线。不能用于几何偏移或介质跨界判定。
    public float3 normal;
};

/// 只选择BSDF/guide的着色法线；几何SurfacePoint保持原始绕序与误差界。
public float3 shading_normal(Hit hit, SurfaceMaterial material)
{
    return any(material.normal != 0) ? material.normal : hit.surface.normal;
}

/// 一项独立 NEE 策略；pdf<=0 表示该策略没有样本，不查询 visibility。
/// direction 为单位方向；distance 用于 Beer，visibility 可使用非单位方向。
/// emission 可缓存光源值或场景私有延迟求值参数，emission_reference由场景解释。
/// 积分器只将二者透传给light_emission，不解释其编码；实际读取可延后到可见性通过后。
/// 两者不改变 pdf 或覆盖规则；昂贵求值留给通过可见性后的 light_emission。
public struct LightSample
{
    public float3 direction;
    public float3 emission;
    public uint emission_reference;
    public float pdf;
    public float distance;
    public geometry::Ray visibility;
};

/// 非采样天空与独立太阳策略分项；sun_pdf 为当前方向的竞争密度，零表示无竞争。
public struct Environment
{
    public float3 sky;
    public float3 sun;
    public float sun_pdf;
};

/// 不含场景策略的积分设置；预算为 1..64，resolution 为采样器的 log2 分辨率。
public struct Settings
{
    public uint seed;
    public uint budget;
    public uint resolution;
    public uint dielectric_limit;
};

/// callable 交接的紧凑命中、路径和绝对样本身份；不允许嵌套 callable 或 TraceRay。
public struct TransportCall
{
    public Hit hit;
    public PathState path;
    public uint2 pixel;
    public uint sample;
};

/// resolve 的 Capability 与入口匹配；continuous 窄入口的额外条件由场景发布时证明。
/// occluded 须与主查询使用相同覆盖规则；RayQuery 场景自行确认 alpha 候选。
public interface IScene
{
    SurfaceMaterial resolve(Hit hit, uint capability, float4 incident);
    openpbr::Energy energy();
    LightSample sample_sun(geometry::SurfacePoint point, float2 sample);
    LightSample sample_light(geometry::SurfacePoint point, float3 sample);
    float3 light_emission(geometry::SurfacePoint point, LightSample sample, bool solar);
    float emission_pdf(Hit hit, float3 previous);
    bool occluded(geometry::Ray ray);
    Environment environment(float3 direction, float3 previous, uint depth);
};

/// 组合 callable 的材料翻译；Flow 为编译期能力掩码，与旧 CAP 参数不混用。
/// 适配器只读取该能力需要的参数和纹理；资源世代保证实际材料落在已证明支持域。
public interface IFlowScene : IScene
{
    SurfaceMaterial resolve_flow(Hit hit, uint flow, float4 incident);
};

/// 主查询仅供 raygen 使用；shade 只选择一次叶子 callable，resolve_hit 只调整实际能力。
/// prefix=true 须证明当前表面为无发光的 delta solid dielectric，否则交给 shade。
public interface ITraceScene : IScene
{
    bool intersect(float3 origin, float3 direction, out Hit hit);
    void resolve_hit(inout Hit hit, float4 incident);
    bool prefix(Hit hit);
    void shade(inout TransportCall data);
};

/// K1 首个实际命中的只读观察者；在 delta 前缀消费之前回调，环境只在主射线未命中回调。
/// surface 输入已 resolve_hit，depth=0；不得更改路径或消费共享随机域。
/// 适配器可在 raygen 内独立查询辅助射线，不得嵌套 callable。
/// 不声称提供光学链重建或镜面命中距离；这些需要独立的、明确的追踪策略。
public interface IPrefixGuides
{
    void surface(TransportCall data);
    void environment(TransportCall data);
};

/// 默认不生成辅助量；编译期专门化后不保留额外资源、材质翻译或存储。
public struct NoPrefixGuides : IPrefixGuides
{
    public void surface(TransportCall data)
    {
    }
    public void environment(TransportCall data)
    {
    }
};
}
