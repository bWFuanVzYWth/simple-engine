/// ReSTIR PT 场景边界；距离为场景单位，RGB 沿用宿主的线性色域。
/// loadSurface 必须从稳定 HitInfo 恢复同一几何，不能将已失效的 primitive 身份解释为新版本。
/// 历史几何已经失效时返回 hit 无效的零 Surface，由 shift 局部拒绝该路径。
module scene;

import restir_pt.types;
import geometry.ray;

namespace restir_pt
{
/// 单次求值的表面；不写入 reservoir，材质闭包由宿主的专用 callable 处理。
public struct Surface
{
    public HitInfo hit;
    public geometry::SurfacePoint point;
    public float3 wo;
    /// loadSurfaceFromOrigin 的观察位置到表面的距离；primary 的 Beer 段由此恢复。
    public float distance;
    public float3 emission;
    public float4 incident;
    public float4 transmitted;
    public float roughness;
    public uint material;
    /// 宿主的已解码命中/纹素缓存；通用算法不解释其编码。
    public uint4 identity;
    /// 原始资源能力；不能因临时推断的 incident 提前覆盖，sample/eval 才按真实介质解析。
    public uint capability;
    public float4 texel;
    public bool frontFacing;
    public bool supportBSDFIndexing;
    public bool supportSimpleRoughnessThreshold;
    /// 是否存在连续 lobe；由专用 Flow 与局部参数确定，不为此调用完整 BSDF。
    public bool hasNonDeltaLobes;
};

/// 连续 BSDF 的 value 含出射方向余弦；pdf 为完整混合分布的立体角密度。
public struct BsdfEvaluation
{
    public float3 value;
    public float pdf;
};

/// 重连第一端的材质求值；介质选择在同一专用材质阶段完成，不留到公共shift状态机。
public struct ConnectionEvaluation
{
    public BsdfEvaluation bsdf;
    public float3 transmittance;
};

/// 发光命中端的专用结果；反向灯PDF与该端的材质/alias键一并求值。
public struct EmitterEvaluation
{
    public float3 emission;
    public float pdf;
    public float3 position;
    public float3 normal;
};

/// sample 的 weight 含 response / probability；delta 的 pdf 是离散概率质量。
public struct BsdfSample
{
    public float3 direction;
    public float3 weight;
    public float4 medium;
    public float pdf;
    /// 被选 component 的条件密度；支持 indexing 时由同次 sample 返回，否则等于完整 pdf。
    public float componentPdf;
    public float relativeEta;
    public uint component;
    public bool delta;
    public bool transmission;
    public bool rough;
};

/// 初采单次专用材质调用的输出；sample 与至多两个加性NEE共享同一闭包。
public struct InitialShading
{
    public BsdfSample sample;
    public BsdfEvaluation nee;
    public float3 emission;
    public BsdfEvaluation secondNEE;
};

/// 光源样本；hit 保存可重建的几何光源身份，环境或方向光由 lightType 区分。
public struct LightSample
{
    public HitInfo hit;
    public float3 direction;
    public float3 radiance;
    public float distance;
    public float pdf;
    public uint lightType;
};

public static const uint INITIAL_SAMPLE = 1u;
public static const uint INITIAL_NEE = 2u;
public static const uint INITIAL_MIS = 4u;
public static const uint INITIAL_RR = 8u;
public static const uint INITIAL_TWO_LIGHTS = 16u;
public static const uint INITIAL_PRIMARY = 32u;
public static const uint INITIAL_POINT_LIGHT = 0x80000000u;

/// 固定bounce身份与显式操作标记；材质阶段按需产生随机维，不携带已采样灯载荷。
public struct InitialRequest
{
    public uint bounceSeed;
    public uint flags;
    public float3 previousPosition;
};

/// 80B数值NEE候选；pdf<=0表示无可见贡献。只保留RIS和重连实际消费的字段。
/// lightType最高位表示有限point光，其余位保持LIGHT_*枚举；geometryFactor已在几何端计算。
public struct InitialDirect
{
    public HitInfo hit;
    public float3 direction;
    public float3 radiance;
    public float3 reflected;
    public float3 attenuatedRadiance;
    public float pdf;
    public float bsdfPdf;
    public float geometryFactor;
    public uint lightType;
};

/// 完整初采材质阶段结果；已完成灯采样、BSDF、介质和可见性，RIS仍由调度器拥有。
public struct InitialVertex
{
    public BsdfSample sample;
    public float3 emission;
    public float emissionPdf;
    public InitialDirect first;
    public InitialDirect second;
};

/// 默认 point-reservoir 所需的精确求交与材料操作。
/// trace、sampleBsdf、evaluateBsdf 仅在 raygen 调用；材料适配器可在叶子 callable 前执行 SER。
/// 重放以源 reservoir 的随机身份为准，previous 只选择几何/光照世代，不改变随机向量。
/// 默认44B ReconnectionData只保存命中和wo；loadSurface须据稳定双侧身份精确恢复前驱介质。
/// 无法从几何恢复介质的场景须另提供实际介质状态，不能把该布局用于任意嵌套体积。
public interface IRestirScene
{
    Surface loadSurface(HitInfo hit, float3 wo, bool previous);
    /// 从观察位置计算 wo；仅恢复一次几何，避免为未知端点重复解码。
    Surface loadSurfaceFromOrigin(HitInfo hit, float3 origin, bool previous);
    /// 光端点只需要几何；不能为可见性或Jacobian执行材质/纹理/着色法线解码。
    geometry::SurfacePoint lightEndpoint(HitInfo hit, bool previous);
    /// 设置真实路径介质并刷新cheap能力/hasNonDelta元数据；不读取BSDF或贴图，不覆盖raw capability。
    void setIncident(inout Surface surface, float4 medium);
    bool trace(float3 origin, float3 direction, bool previous, out HitInfo hit, out float distance);
    bool visible(Surface source, Surface target, bool previous);
    bool visibleDirection(Surface source, float3 direction, bool previous);
    /// 按实际出射侧选择均匀介质，返回该无交点段的 Beer 衰减；不执行可见性查询。
    float3 transmittance(Surface source, float3 direction, float distance);
    /// 与场景的环境追踪边界相同，不能由复用算法另设较短截断。
    float environmentDistance();
    BsdfSample sampleBsdf(Surface surface, float3 u);
    /// sampleContinuation=false只求相同NEE与emission，不计算之后必被RR丢弃的散射。
    /// NEE响应/PDF与emission不得依赖bsdfU或采样结果；该标记不改变路径随机域。
    InitialShading shadeInitial(Surface surface, float3 bsdfU, float3 neeDirection,
                                float3 secondNEEDirection, bool sampleContinuation);
    /// 完整的Flow专用初采；选灯、材料解码、支持域、阴影与散射只能在该材质阶段执行一次。
    InitialVertex shadeInitialVertex(Surface surface, InitialRequest request);
    BsdfEvaluation evaluateBsdf(Surface surface, float3 wi, int component);
    /// 与evaluateBsdf相同的散射，加上实际连接段Beer；distance>=0，不能改变提案PDF。
    ConnectionEvaluation evaluateConnection(Surface surface, float3 wi, float distance,
                                            int component);
    /// 发光端材料、几何与反向PDF的一次专用求值；消费surface.hit与实际入射wo，from是前驱位置。
    /// 非发光返回零；此时position/normal不再被消费者读取，不改变材料或灯分布。
    EmitterEvaluation evaluateEmitter(Surface surface, float3 from, bool previous);
    bool isSampleRough(Surface surface, uint component, float pdf, float threshold);
    /// 同一场景世代、完整surface与u须返回相同结果；允许在shadeInitial前后重算。
    /// 不消费隐含随机流，不依赖调用次数或着色反馈；不能修改当前场景的采样分布。
    /// 初采双族模式把u.z分别限制在[0,0.5)和[0.5,1)，各返回该族条件PDF；
    /// 缺失族返回零，不能改投其他族。lightPdf必须返回同一条件密度。
    LightSample sampleLight(Surface surface, float3 u);
    float lightPdf(Surface source, HitInfo light, float3 lightDirection, uint lightType,
                   bool previous);
    /// 在实际最后顶点求环境；主 miss 使用相机位置，重连环境端点使用新的前驱位置。
    float3 environment(float3 direction, float3 origin, bool previous);
    /// 有限 analytic 光是否为 delta 位置源；几何光源与方向光返回 false。
    bool isPointLight(HitInfo light);
    float3 initialOrigin(uint2 pixel);
    float3 initialDirection(uint2 pixel);
    float4 initialMedium(uint2 pixel);
    float primaryRayFootprint(Surface surface, uint2 pixel, bool previous);
};
}
