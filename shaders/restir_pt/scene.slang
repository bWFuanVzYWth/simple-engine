/// ReSTIR PT 场景边界；距离为场景单位，RGB 沿用宿主的线性色域。
/// 当前已冻结应用集成；保留引擎算法。状态、实验依据与适用限制见 restir_pt crate 模块文档。
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
    public var hit : HitInfo;
    public var point : geometry::SurfacePoint;
    public var wo : float3;
    /// loadSurfaceFromOrigin 的观察位置到表面的距离；primary 的 Beer 段由此恢复。
    public var distance : float;
    public var emission : float3;
    public var incident : float4;
    public var transmitted : float4;
    public var roughness : float;
    public var material : uint;
    /// 宿主的已解码命中/纹素缓存；通用算法不解释其编码。
    public var identity : uint4;
    /// 原始资源能力；不能因临时推断的 incident 提前覆盖，sample/eval 才按真实介质解析。
    public var capability : uint;
    public var texel : float4;
    public var frontFacing : bool;
    public var supportBSDFIndexing : bool;
    public var supportSimpleRoughnessThreshold : bool;
    /// 是否存在连续 lobe；由专用 Flow 与局部参数确定，不为此调用完整 BSDF。
    public var hasNonDeltaLobes : bool;
};

/// 连续 BSDF 的 value 含出射方向余弦；pdf 为完整混合分布的立体角密度。
public struct BsdfEvaluation
{
    public var value : float3;
    public var pdf : float;
};

/// 重连第一端的材质求值；介质选择在同一专用材质阶段完成，不留到公共shift状态机。
public struct ConnectionEvaluation
{
    public var bsdf : BsdfEvaluation;
    public var transmittance : float3;
    /// 第二端BSDF只消费连接段IOR；Beer已在transmittance中，固定后缀不重算。
    public var incidentIor : float;
};

/// 发光命中端的专用结果；反向灯PDF与该端的材质/alias键一并求值。
public struct EmitterEvaluation
{
    public var emission : float3;
    public var pdf : float;
    public var position : float3;
    public var normal : float3;
};

/// sample 的 weight 含 response / probability；delta 的 pdf 是离散概率质量。
public struct BsdfSample
{
    public var direction : float3;
    public var weight : float3;
    public var medium : float4;
    public var pdf : float;
    /// 被选 component 的条件密度；支持 indexing 时由同次 sample 返回，否则等于完整 pdf。
    public var componentPdf : float;
    public var relativeEta : float;
    public var component : uint;
    public var delta : bool;
    public var transmission : bool;
    public var rough : bool;
};

/// 初采单次专用材质调用的输出；sample 与至多两个加性NEE共享同一闭包。
public struct InitialShading
{
    public var sample : BsdfSample;
    public var nee : BsdfEvaluation;
    public var emission : float3;
    public var secondNEE : BsdfEvaluation;
};

/// 光源样本；hit 保存可重建的几何光源身份，环境或方向光由 lightType 区分。
public struct LightSample
{
    public var hit : HitInfo;
    public var direction : float3;
    public var radiance : float3;
    public var distance : float;
    public var pdf : float;
    public var lightType : uint;
};

public static const let INITIAL_SAMPLE = 1u;
public static const let INITIAL_NEE = 2u;
public static const let INITIAL_MIS = 4u;
public static const let INITIAL_RR = 8u;
public static const let INITIAL_TWO_LIGHTS = 16u;
public static const let INITIAL_PRIMARY = 32u;
public static const let INITIAL_POINT_LIGHT = 0x80000000u;

/// 固定bounce身份与显式操作标记；材质阶段按需产生随机维，不携带已采样灯载荷。
public struct InitialRequest
{
    public var bounceSeed : uint;
    public var flags : uint;
    public var previousPosition : float3;
    public var throughput : float3;
    public var weight : float;
    /// 负值表示上一个散射没有竞争的命中灯 MIS。
    public var previousMisPdf : float;
};

/// 80B数值NEE候选；pdf<=0表示无可见贡献。只保留RIS和重连实际消费的字段。
/// lightType最高位表示有限point光，其余位保持LIGHT_*枚举；geometryFactor已在几何端计算。
public struct InitialDirect
{
    public var hit : HitInfo;
    public var direction : float3;
    public var radiance : float3;
    public var reflected : float3;
    public var attenuatedRadiance : float3;
    public var pdf : float;
    public var bsdfPdf : float;
    public var geometryFactor : float;
    public var lightType : uint;
};

/// 材质阶段更新路径内 RIS 权重并返回胜出的候选；调度器据此写入重连状态。
public struct InitialVertex
{
    public var sample : BsdfSample;
    public var emission : float3;
    public var emissionPdf : float;
    public var direct : InitialDirect;
    public var weight : float;
};

/// 默认 point-reservoir 所需的精确求交与材料操作。
/// trace、sampleBsdf、evaluateBsdf 仅在 raygen 调用；材料适配器可在叶子 callable 前执行 SER。
/// 重放以源 reservoir 的随机身份为准，previous 只选择几何/光照世代，不改变随机向量。
/// ReconnectionData保存前缀实际携带的介质；loadSurface只提供表面元数据与默认侧面状态，
/// 不能用表面的面朝向代替路径介质。任意嵌套体积仍须由场景适配器维护其边界契约。
public interface IRestirScene
{
    func loadSurface(hit: HitInfo, wo: float3, previous: bool)->Surface;
    /// 从观察位置计算 wo；仅恢复一次几何，避免为未知端点重复解码。
    func loadSurfaceFromOrigin(hit: HitInfo, origin: float3, previous: bool)->Surface;
    /// 光端点只需要几何；不能为可见性或Jacobian执行材质/纹理/着色法线解码。
    func lightEndpoint(hit: HitInfo, previous: bool)->geometry::SurfacePoint;
    /// 已知发光端点在当前wo侧是否支持发光；与初采的侧面/覆盖规则相同。
    /// 固定端点的纹素和发光强度已缓存，不得为侧面判断重复解码或采样。
    func emissionSideSupported(surface: Surface)->bool;
    /// 设置真实路径介质并刷新cheap能力/hasNonDelta元数据；不读取BSDF或贴图，不覆盖raw capability。
    func setIncident(inout surface: Surface, medium: float4)->void;
    func trace(origin: float3, direction: float3, previous: bool, out hit: HitInfo,
               out distance: float)
        ->bool;
    func visible(source: Surface, target: Surface, previous: bool)->bool;
    func visibleDirection(source: Surface, direction: float3, previous: bool)->bool;
    /// 按实际出射侧选择均匀介质，返回该无交点段的 Beer 衰减；不执行可见性查询。
    func transmittance(source: Surface, direction: float3, distance: float)->float3;
    /// 与场景的环境追踪边界相同，不能由复用算法另设较短截断。
    func environmentDistance()->float;
    func sampleBsdf(surface: Surface, u: float3)->BsdfSample;
    /// sampleContinuation=false只求相同NEE与emission，不计算之后必被RR丢弃的散射。
    /// NEE响应/PDF与emission不得依赖bsdfU或采样结果；该标记不改变路径随机域。
    func shadeInitial(surface: Surface, bsdfU: float3, neeDirection: float3,
                      secondNEEDirection: float3, sampleContinuation: bool)
        ->InitialShading;
    /// 完整的Flow专用初采；选灯、材料解码、支持域、阴影与散射只能在该材质阶段执行一次。
    func shadeInitialVertex(surface: Surface, request: InitialRequest)->InitialVertex;
    func evaluateBsdf(surface: Surface, wi: float3, component: int)->BsdfEvaluation;
    /// 与evaluateBsdf相同的散射，加上实际连接段Beer；distance>=0，不能改变提案PDF。
    func evaluateConnection(surface: Surface, wi: float3, distance: float, component: int)
        ->ConnectionEvaluation;
    /// 发光端材料、几何与反向PDF的一次专用求值；消费surface.hit与实际入射wo，from是前驱位置。
    /// 非发光返回零；此时position/normal不再被消费者读取，不改变材料或灯分布。
    func evaluateEmitter(surface: Surface, from: float3, previous: bool)->EmitterEvaluation;
    func isSampleRough(surface: Surface, component: uint, pdf: float, threshold: float)->bool;
    /// 同一场景世代、完整surface与u须返回相同结果；允许在shadeInitial前后重算。
    /// 不消费隐含随机流，不依赖调用次数或着色反馈；不能修改当前场景的采样分布。
    /// 初采双族模式把u.z分别限制在[0,0.5)和[0.5,1)，各返回该族条件PDF；
    /// 缺失族返回零，不能改投其他族。lightPdf必须返回同一条件密度。
    func sampleLight(surface: Surface, u: float3)->LightSample;
    func lightPdf(source: Surface, light: HitInfo, lightDirection: float3, lightType: uint,
                  previous: bool)
        ->float;
    /// 在实际最后顶点求环境；主 miss 使用相机位置，重连环境端点使用新的前驱位置。
    func environment(direction: float3, origin: float3, previous: bool)->float3;
    /// 有限 analytic 光是否为 delta 位置源；几何光源与方向光返回 false。
    func isPointLight(light: HitInfo)->bool;
    func initialOrigin(pixel: uint2)->float3;
    func initialDirection(pixel: uint2)->float3;
    func initialMedium(pixel: uint2)->float4;
    func primaryRayFootprint(surface: Surface, pixel: uint2, previous: bool)->float;
};
}
