/// 编译期场景适配契约。长度统一为场景单位，颜色为线性 BT.709；资源由宿主保活。
module scene;

import geometry.ray;
import openpbr.api;
import light_sampling.sample;
import transport.surface_material;

namespace pt {
public static const let PATH_ACTIVE = 0u;
public static const let PATH_DONE = 1u;
public static const let PATH_INVALID = 2u;
public static const let PATH_DIELECTRIC_LIMIT = 3u;
public static const let FAR = 1.0e8;

/// 紧凑命中缓存；identity、texel 的编码仅由场景解释，不保存完整 OpenPBR 材料。
public struct Hit {
    public var surface : geometry::SurfacePoint;
    public var identity : uint4;
    /// 场景的紧凑程序分类，由 prefix／shade 解释，不等同于 OpenPBR Capability。
    public var capability : uint;
    public var distance : float;
    public var area : float;
    public var front : bool;
    public var texel : float4;
};

/// 路径唯一可变状态；depth 为已消费顶点数，previous_pdf=0 表示离散前驱。
/// 非 ACTIVE 输出的 origin、previous、previous_pdf 规范为零，不再携带下一段或 MIS 身份。
public struct PathState {
    public var origin : float3;
    public var direction : float3;
    public var throughput : float3;
    public var radiance : float3;
    public var medium : float4;
    public var previous : float3;
    public var previous_pdf : float;
    public var eta_scale : float;
    public var depth : uint;
    public var dielectric_events : uint;
    public var status : uint;
};

/// 只选择BSDF/guide的着色法线；几何SurfacePoint保持原始绕序与误差界。
public func shading_normal(hit: Hit, material: transport::SurfaceMaterial)->float3 {
    return any(material.normal != 0) ? material.normal : hit.surface.normal;
}

/// 非采样天空与独立太阳策略分项；sun_pdf 为当前方向的竞争密度，零表示无竞争。
public struct Environment {
    public var sky : float3;
    public var sun : float3;
    public var sun_pdf : float;
};

/// 不含场景策略的积分设置；budget为1..64次表面散射，末顶点仍计算发光与NEE。
/// resolution为采样器的log2分辨率。
public struct Settings {
    public var seed : uint;
    public var budget : uint;
    public var resolution : uint;
    public var dielectric_limit : uint;
    /// 对eta补偿后的吞吐量使用平方根轮盘；仅改变存活概率，存活路径仍除以该概率。
    public var square_root_roulette : bool;
};

/// callable 交接的紧凑命中、路径和绝对样本身份；不允许嵌套 callable 或 TraceRay。
public struct TransportCall {
    public var hit : Hit;
    public var path : PathState;
    public var pixel : uint2;
    public var sample : uint;
};

/// resolve 的 Capability 与入口匹配；continuous 窄入口的额外条件由场景发布时证明。
/// occluded 须与主查询使用相同覆盖规则；RayQuery 场景自行确认 alpha 候选。
public interface IScene {
    func resolve(hit: Hit, capability: uint, incident: float4)->transport::SurfaceMaterial;
    func energy()->openpbr::Energy;
    func sample_sun(point_value: geometry::SurfacePoint, sample_value: float2)
        ->light_sampling::LightSample;
    func sample_light(point_value: geometry::SurfacePoint, sample_value: float3)
        ->light_sampling::LightSample;
    func light_emission(point_value: geometry::SurfacePoint,
                        sample_value: light_sampling::LightSample, solar: bool)
        ->float3;
    func emission_pdf(hit: Hit, previous: float3)->float;
    func occluded(ray: geometry::Ray)->bool;
    func environment(direction: float3, previous: float3, depth: uint)->Environment;
};

/// 组合 callable 的材料翻译；flow 为编译期能力掩码，与旧 CAP 参数不混用。
/// 适配器只读取该能力需要的参数和纹理；资源世代保证实际材料落在已证明支持域。
public interface IFlowScene : IScene {
    func resolve_flow(hit: Hit, flow: uint, incident: float4)->transport::SurfaceMaterial;
};

/// 非发光delta solid的唯一界面数据；不存在颜色、粗糙度或连续lobe状态。
public struct DeltaInterface {
    public var normal : float3;
    public var transmitted : float4;
    public var filter : float3;
};
public interface IDeltaPrefixScene {
    func delta_interface(hit: Hit, incident: float4)->DeltaInterface;
};

/// 主查询仅供 raygen 使用；shade 只选择一次叶子 callable，resolve_hit 只调整实际能力。
/// prefix=true 须证明当前表面为无发光的 delta solid dielectric，否则交给 shade。
public interface ITraceScene : IScene, IDeltaPrefixScene {
    func intersect(origin: float3, direction: float3, out hit: Hit)->bool;
    func resolve_hit(inout hit: Hit, incident: float4)->void;
    func prefix(hit: Hit)->bool;
    func shade(inout data: TransportCall)->void;
};

/// K1 首个实际命中的只读观察者；在 delta 前缀消费之前回调，环境只在主射线未命中回调。
/// surface 输入已 resolve_hit，depth=0；不得更改路径或消费共享随机域。
/// 适配器可在 raygen 内独立查询辅助射线，不得嵌套 callable。
/// 不声称提供光学链重建或镜面命中距离；这些需要独立的、明确的追踪策略。
public interface IPrefixGuides {
    func surface(data: TransportCall)->void;
    func environment(data: TransportCall)->void;
};

/// HitObject路径的首命中观察者；复用实际命中，不为材质解码重查同一主射线。
public interface IHitGuides : IPrefixGuides {
    func surface_hit(object: HitObject, data: TransportCall)->void;
};

/// 默认不生成辅助量；编译期专门化后不保留额外资源、材质翻译或存储。
public struct NoPrefixGuides : IHitGuides {
    public func surface(data: TransportCall)->void {
    }
    public func environment(data: TransportCall)->void {
    }
    public func surface_hit(object: HitObject, data: TransportCall)->void {
    }
};
}
