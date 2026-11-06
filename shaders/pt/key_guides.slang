/// 按材质键分流的导引；材料求值只在专用命中/known-surface程序，PSR只消费数值。
module key_guides;
import pt.scene;
import pt.reconstruction;
import pt.denoise;
import bsdf.openpbr.flow;
import geometry.ray;

namespace pt
{
public static const uint KEY_GUIDE_DISCRETE = 1u;
public static const uint KEY_GUIDE_REFLECTION = 2u;
public static const uint KEY_GUIDE_REFRACTED = 4u;
public static const uint KEY_GUIDE_POSE_KNOWN = 8u;
public static const uint KEY_GUIDE_POSE_FIXED = 16u;

/// 128B无损数值顶点；无完整PBR闭包、材质身份、纹素、随机状态或光源。
/// normal_roughness.xyz保留实际着色法线朝向；输出投影最后朝向相机。
public struct KeyGuideVertex
{
    public geometry::SurfacePoint point;
    public float distance;
    public float4 normal_roughness;
    public float3 diffuse;
    public float3 specular;
    public float3 canonicalDirection;
    public float3 reflectionDirection;
    public float4 medium;
    public float3 previousPosition;
    public uint flags;
};

/// 命中程序输入仅medium为当前介质，输出复用全128B。
/// known-surface适配器可在未使用输入字段中编码稳定命中与观察方向，须先消费再写输出。
public struct KeyGuideCall
{
    public KeyGuideVertex value;
    public static KeyGuideCall input(float4 incident)
    {
        KeyGuideCall result = {};
        result.value.medium = incident;
        return result;
    }
};

/// 材质已按guide操作投影解码；不消费发光、alias、边界颜色filter或任何光照策略。
public KeyGuideVertex make_key_guide<let Flow : uint>(Hit hit, SurfaceMaterial material,
                                                      openpbr::Energy energy, GuidePose pose,
                                                      float3 incoming, float4 incident)
{
    float3 normal = shading_normal(hit, material);
    DenoiseGuides albedos =
        denoise_surface_flow<Flow>(energy, material, normal, -incoming, incident);
    bool reflection = true, refracted = false;
    float3 next = reflect(incoming, normal);
    static const bool Delta =
        (Flow & (openpbr::FLOW_DELTA_REFLECTION | openpbr::FLOW_DELTA_TRANSMISSION)) != 0;
    if (Delta && (Flow & (openpbr::FLOW_SOLID_TRANSMISSION | openpbr::FLOW_THIN_TRANSMISSION)) != 0)
        next = guide_delta_direction(material, normal, incoming, incident, reflection, refracted);
    bool discrete =
        openpbr::is_discrete_only_flow<Flow>(material.pbr, incident, material.transmitted);
    float4 medium =
        !reflection && (material.pbr.control.x & 2u) == 0 ? material.transmitted : incident;
    uint flags = (discrete ? KEY_GUIDE_DISCRETE : 0u) | (reflection ? KEY_GUIDE_REFLECTION : 0u) |
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
    bool intersect_guide(float3 origin, float3 direction, float4 incident,
                         out KeyGuideVertex vertex);
};

/// 兼容原有投影记录，只在endpoint.material中使用数值着色法线，不再从记录求BSDF或albedo。
public struct KeyGuideEndpoint
{
    public GuideEndpoint endpoint;
    public DenoiseGuides albedos;
};

void set_key_guide_vertex(inout KeyGuideEndpoint result, KeyGuideVertex vertex)
{
    result.endpoint.hit.surface = vertex.point;
    result.endpoint.hit.distance = vertex.distance;
    result.endpoint.material.normal = vertex.normal_roughness.xyz;
    result.endpoint.pose = { vertex.previousPosition, (vertex.flags & KEY_GUIDE_POSE_KNOWN) != 0,
                             (vertex.flags & KEY_GUIDE_POSE_FIXED) != 0 };
    result.albedos = { vertex.normal_roughness, vertex.diffuse, vertex.specular };
}

/// 与trace_guide相同的预算、PSR、候选回退及查询计数，首面材料只解码一次。
/// firstReflection只选择首面的理想反射数值，不改变后续材质/随机域，不重解首面。
public KeyGuideEndpoint trace_key_guide<Scene : IKeyGuideTrace>(Scene scene, KeyGuideVertex first,
                                                                float3 cameraOrigin,
                                                                float3 cameraDirection,
                                                                float4 initialMedium, uint budget,
                                                                bool firstReflection)
{
    KeyGuideEndpoint result = {};
    result.endpoint.incident = initialMedium;
    result.endpoint.direction = cameraDirection;
    result.endpoint.camera_origin = cameraOrigin;
    result.endpoint.camera_direction = cameraDirection;
    result.endpoint.psr = psr_initial();
    result.endpoint.queries = 1;
    set_key_guide_vertex(result, first);
    KeyGuideEndpoint candidate = result;
    KeyGuideVertex vertex = first;
    while (true)
    {
        bool discrete = (vertex.flags & KEY_GUIDE_DISCRETE) != 0;
        if (!discrete && !(firstReflection && result.endpoint.queries == 1))
        {
            float3 position, normal;
            if (psr_surface(result.endpoint.psr, cameraOrigin, cameraDirection,
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
        bool forced = firstReflection && result.endpoint.queries == 1;
        bool reflection = forced || (vertex.flags & KEY_GUIDE_REFLECTION) != 0;
        bool refracted = !forced && (vertex.flags & KEY_GUIDE_REFRACTED) != 0;
        float3 next = forced ? vertex.reflectionDirection : vertex.canonicalDirection;
        if (result.endpoint.queries == 1 && !reflection && !firstReflection)
        {
            result.endpoint.interface_specular = vertex.specular;
            result.endpoint.use_interface_specular = true;
        }
        result.endpoint.psr =
            psr_append(result.endpoint.psr, vertex.point.position, vertex.normal_roughness.xyz,
                       reflection, refracted, (vertex.flags & KEY_GUIDE_POSE_FIXED) != 0);
        if (refracted)
            result.endpoint.flags |= GUIDE_REFRACTION_PROXY;
        float3 origin = geometry::offset_origin(vertex.point, next);
        if (!reflection)
            result.endpoint.incident = vertex.medium;
        result.endpoint.direction = next;
        ++result.endpoint.queries;
        KeyGuideVertex following;
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
