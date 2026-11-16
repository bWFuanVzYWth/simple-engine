/// 按材质键分流的导引；材料求值只在专用命中/known-surface程序，PSR只消费数值。
module key_guides;
import pt.scene;
import pt.reconstruction;
import pt.denoise;
import bsdf.openpbr.flow;
import geometry.ray;

namespace pt
{
public static const let KEY_GUIDE_DISCRETE = 1u;
public static const let KEY_GUIDE_REFLECTION = 2u;
public static const let KEY_GUIDE_REFRACTED = 4u;
public static const let KEY_GUIDE_POSE_KNOWN = 8u;
public static const let KEY_GUIDE_POSE_FIXED = 16u;

/// 128B无损数值顶点；无完整PBR闭包、材质身份、纹素、随机状态或光源。
/// normal_roughness.xyz保留实际着色法线朝向；输出投影最后朝向相机。
public struct KeyGuideVertex
{
    public var point : geometry::SurfacePoint;
    public var distance : float;
    public var normal_roughness : float4;
    public var diffuse : float3;
    public var specular : float3;
    public var canonical_direction : float3;
    public var reflection_direction : float3;
    public var medium : float4;
    public var previous_position : float3;
    public var flags : uint;
};

/// 命中程序输入仅medium为当前介质，输出复用全128B。
/// known-surface适配器可在未使用输入字段中编码稳定命中与观察方向，须先消费再写输出。
public struct KeyGuideCall
{
    public var value : KeyGuideVertex;
    public static func input(incident: float4)->KeyGuideCall
    {
        var result : KeyGuideCall = {};
        result.value.medium = incident;
        return result;
    }
};

/// 材质已按guide操作投影解码；不消费发光、alias、边界颜色filter或任何光照策略。
public func make_key_guide<let flow : uint>(hit: Hit, material: SurfaceMaterial,
                                            energy: openpbr::Energy, pose: GuidePose,
                                            incoming: float3, incident: float4)
    ->KeyGuideVertex
{
    let normal = shading_normal(hit, material);
    let albedos = denoise_surface_flow<flow>(energy, material, normal, -incoming, incident);
    var reflection = true;
    var refracted = false;
    var next : float3 = reflect(incoming, normal);
    static const let delta
        : bool = (flow & (openpbr::FLOW_DELTA_REFLECTION | openpbr::FLOW_DELTA_TRANSMISSION)) != 0;
    if (delta && (flow & (openpbr::FLOW_SOLID_TRANSMISSION | openpbr::FLOW_THIN_TRANSMISSION)) != 0)
        next = guide_delta_direction(material, normal, incoming, incident, reflection, refracted);
    let discrete =
        openpbr::is_discrete_only_flow<flow>(material.pbr, incident, material.transmitted);
    let medium : float4 = !reflection && (material.pbr.control.x & 2u) == 0 ? material.transmitted
                                                                            : incident;
    let flags
        : uint = (discrete ? KEY_GUIDE_DISCRETE : 0u) | (reflection ? KEY_GUIDE_REFLECTION : 0u) |
                 (refracted ? KEY_GUIDE_REFRACTED : 0u) | (pose.known ? KEY_GUIDE_POSE_KNOWN : 0u) |
                 (pose.fixed ? KEY_GUIDE_POSE_FIXED : 0u);
    return { hit.surface,
             hit.distance,
             float4(normal, albedos.normal_roughness.w),
             albedos.diffuse_albedo,
             albedos.specular_albedo,
             next,
             reflect(incoming, normal),
             medium,
             pose.previous_position,
             flags };
}

/// 每个新命中的原子追踪/专用导引操作，不能先通用读取材质再分流。
public interface IKeyGuideTrace
{
    func intersect_guide(origin: float3, direction: float3, incident: float4,
                         out vertex: KeyGuideVertex)
        ->bool;
};

/// 兼容原有投影记录，只在endpoint.material中使用数值着色法线，不再从记录求BSDF或albedo。
public struct KeyGuideEndpoint
{
    public var endpoint : GuideEndpoint;
    public var albedos : DenoiseGuides;
};

func set_key_guide_vertex(inout result: KeyGuideEndpoint, vertex: KeyGuideVertex)->void
{
    result.endpoint.hit.surface = vertex.point;
    result.endpoint.hit.distance = vertex.distance;
    result.endpoint.material.normal = vertex.normal_roughness.xyz;
    result.endpoint.pose = { vertex.previous_position, (vertex.flags & KEY_GUIDE_POSE_KNOWN) != 0,
                             (vertex.flags & KEY_GUIDE_POSE_FIXED) != 0 };
    result.albedos = { vertex.normal_roughness, vertex.diffuse, vertex.specular };
}

/// 与trace_guide相同的预算、PSR、候选回退及查询计数，首面材料只解码一次。
/// firstReflection只选择首面的理想反射数值，不改变后续材质/随机域，不重解首面。
public func trace_key_guide<Scene : IKeyGuideTrace>(scene: Scene, first: KeyGuideVertex,
                                                    camera_origin: float3, camera_direction: float3,
                                                    initial_medium: float4, budget: uint,
                                                    first_reflection: bool,
                                                    delta_only: bool = false)
    ->KeyGuideEndpoint
{
    var result : KeyGuideEndpoint = {};
    result.endpoint.incident = initial_medium;
    result.endpoint.direction = camera_direction;
    result.endpoint.camera_origin = camera_origin;
    result.endpoint.camera_direction = camera_direction;
    result.endpoint.psr = psr_initial();
    result.endpoint.queries = 1;
    set_key_guide_vertex(result, first);
    var candidate : KeyGuideEndpoint = result;
    var vertex : KeyGuideVertex = first;
    while (true)
    {
        let discrete : bool = (vertex.flags & KEY_GUIDE_DISCRETE) != 0;
        if (!discrete && (delta_only || !(first_reflection && result.endpoint.queries == 1)))
        {
            var position : float3;
            var normal : float3;
            if (psr_surface(result.endpoint.psr, camera_origin, camera_direction,
                            vertex.point.position, vertex.point.normal, vertex.normal_roughness.xyz,
                            position, normal))
            {
                if (result.endpoint.use_interface_specular)
                    result.albedos.specular_albedo = result.endpoint.interface_specular;
                return result;
            }
            candidate.endpoint.flags |= GUIDE_SURFACE_PROXY;
            return candidate;
        }
        if (result.endpoint.queries == budget)
        {
            candidate.endpoint.flags |= GUIDE_SURFACE_PROXY | GUIDE_BUDGET_PROXY;
            return candidate;
        }
        let forced : bool = first_reflection && result.endpoint.queries == 1;
        let reflection : bool = forced || (vertex.flags & KEY_GUIDE_REFLECTION) != 0;
        let refracted : bool = !forced && (vertex.flags & KEY_GUIDE_REFRACTED) != 0;
        let next = forced ? vertex.reflection_direction : vertex.canonical_direction;
        if (result.endpoint.queries == 1 && !reflection && !first_reflection)
        {
            result.endpoint.interface_specular = vertex.specular;
            result.endpoint.use_interface_specular = true;
        }
        result.endpoint.psr =
            psr_append(result.endpoint.psr, vertex.point.position, vertex.normal_roughness.xyz,
                       reflection, refracted, (vertex.flags & KEY_GUIDE_POSE_FIXED) != 0);
        if (refracted)
            result.endpoint.flags |= GUIDE_REFRACTION_PROXY;
        let origin = geometry::offset_origin(vertex.point, next);
        if (!reflection)
            result.endpoint.incident = vertex.medium;
        result.endpoint.direction = next;
        ++result.endpoint.queries;
        var following : KeyGuideVertex;
        if (!scene.intersect_guide(origin, next, result.endpoint.incident, following))
        {
            result.endpoint.flags |= GUIDE_SKY;
            if (result.endpoint.psr.count != 0)
                result.endpoint.flags |= GUIDE_MOTION_PROXY;
            return result;
        }
        vertex = following;
        result.endpoint.distance += vertex.distance;
        set_key_guide_vertex(result, vertex);
    }
}
}
