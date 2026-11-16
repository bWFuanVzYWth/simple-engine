/// 与照明独立的规范几何 guide；不计算辐亮度、Fresnel 抽样、Beer 或 roulette。
module reconstruction;

import pt.denoise;
import pt.scene;
import geometry.ray;
import bsdf.openpbr.api;
import "bsdf/openpbr/math/microfacet_energy.slang";

namespace pt
{
/// 真正逃逸；光学链天空采用原相机方向代理，不声称恢复逆光学映射。
public static const let GUIDE_SKY = 1u;
/// 实际接口代替无法取得或无法投影的规范终点。
public static const let GUIDE_SURFACE_PROXY = 2u;
/// 缺少对象或整条光学链的前态，使用当前虚拟面的相机运动。
public static const let GUIDE_MOTION_PROXY = 4u;
/// 查询预算耗尽，保留第一个可用接口；不是非 delta 终点或天空。
public static const let GUIDE_BUDGET_PROXY = 8u;
/// 没有有效几何；调用方不能将有限占位声明为真实 guide。
public static const let GUIDE_INVALID = 16u;
/// 折射终点采用切平面代理，未恢复 inverse Snell 光学对应。
public static const let GUIDE_REFRACTION_PROXY = 32u;

/// 当前物理命中的前态；known 要求同一几何身份和重心坐标，fixed 指接口平面固定。
/// 未知时 previous_position 不参与计算，不能从相似几何猜测对应关系。
public struct GuidePose
{
    public var previous_position : float3;
    public var known : bool;
    public var fixed : bool;
};

/// guide 查询共享场景几何和材料翻译；姿态由掌握源身份的上层提供。
/// intersect 不消费照明随机域；alpha 的独立确定性规则由适配器维护。
public interface IGuideScene
{
    func intersect(origin: float3, direction: float3, out hit: Hit)->bool;
    func resolve_hit(inout hit: Hit, incident: float4)->void;
    /// 翻译当前完整闭包；hit.capability 是场景程序编号，不能当作 OpenPBR Capability。
    func guide_material(hit: Hit, incident: float4)->SurfaceMaterial;
    func energy()->openpbr::Energy;
    func guide_pose(hit: Hit)->GuidePose;
};

/// 相机侧展开 F(x)=A*x+t；rotation 为单位 quaternion，odd 记录反射奇偶。
/// 平面镜反射精确，折射仅保留终点切平面代理；不提供 inverse Snell 对应。
public struct Psr
{
    public var rotation : float4;
    public var translation : float3;
    public var count : uint;
    public var odd : bool;
    public var refracted : bool;
    public var fixed : bool;
};

/// 无光学前缀的恒等变换。
public func psr_initial()->Psr
{
    return { float4(0, 0, 0, 1), float3(0), 0, false, false, true };
}

/// 将世界方向映射到相机侧虚拟空间，不含平移。
public func psr_direction(psr: Psr, direction: float3)->float3
{
    let twice = 2 * cross(psr.rotation.xyz, direction);
    let rotated = direction + psr.rotation.w * twice + cross(psr.rotation.xyz, twice);
    return psr.odd ? -rotated : rotated;
}

/// 将物理世界位置映射到相机侧虚拟空间。
public func psr_point(psr: Psr, position: float3)->float3
{
    return psr_direction(psr, position) + psr.translation;
}

/// 追加真实接口事件；normal 须为产生方向的单位光学法线，安全偏移不参与变换。
public func psr_append(psr: Psr, position: float3, normal: float3, reflection: bool,
                       refracted: bool, fixed: bool)
    ->Psr
{
    var psr = psr;
    if (reflection)
    {
        psr.translation += psr_direction(psr, 2 * normal * dot(normal, position));
        let a = psr.rotation;
        var rotation = float4(a.w * normal + cross(a.xyz, normal), -dot(a.xyz, normal));
        psr.rotation = normalize(rotation);
        psr.odd = !psr.odd;
    }
    ++psr.count;
    psr.refracted = psr.refracted || refracted;
    psr.fixed = psr.fixed && fixed;
    return psr;
}

/// 展开终点平面并与实际采样相机射线求交，返回一致的深度/运动锚点和法线。
/// 折射为切平面近似；掠射、背向或非有限平面返回 false，不伪造天空。
public func psr_surface(psr: Psr, origin: float3, direction: float3, position: float3,
                        geometric_normal: float3, shading_normal: float3,
                        out virtual_position: float3, out virtual_normal: float3)
    ->bool
{
    virtual_position = position;
    virtual_normal = shading_normal;
    if (psr.count != 0)
    {
        let plane = psr_point(psr, position);
        let normal = psr_direction(psr, geometric_normal);
        virtual_normal = psr_direction(psr, shading_normal);
        let denominator = dot(normal, direction);
        if (!(abs(denominator) > 1e-4))
            return false;
        let distance = dot(normal, plane - origin) / denominator;
        if (!(distance > 0))
            return false;
        virtual_position = origin + direction * distance;
    }
    return all(isfinite(virtual_position)) && all(isfinite(virtual_normal)) &&
           dot(virtual_normal, virtual_normal) > 1e-12;
}

/// 规范终点和其局部材料；仅为单次 raygen 的临时值，不规定全屏缓存 ABI。
public struct GuideEndpoint
{
    public var hit : Hit;
    public var material : SurfaceMaterial;
    public var incident : float4;
    public var direction : float3;
    public var pose : GuidePose;
    public var psr : Psr;
    public var camera_origin : float3;
    public var camera_direction : float3;
    /// 首个物理表面之后的段长度；不覆盖原相机空气段长度。
    public var distance : float;
    public var queries : uint;
    public var flags : uint;
    public var interface_specular : float3;
    public var use_interface_specular : bool;
};

/// 只按几何和实际 IOR 选择规范事件：优先透射，TIR/conductor 反射。
/// 材料须已证明整个闭包为 delta；不读取响应、吸收或随机概率。
public func guide_delta_direction(material: SurfaceMaterial, normal: float3, direction: float3,
                                  incident: float4, out reflection: bool, out refracted: bool)
    ->float3
{
    var normal = normal;
    reflection = true;
    refracted = false;
    normal = dot(normal, direction) > 0 ? -normal : normal;
    if ((material.pbr.control.x & 1u) != 0)
    {
        if ((material.pbr.control.x & 2u) != 0)
        {
            reflection = false;
            return direction;
        }
        let transmitted : float3 = refract(direction, normal, incident.x / material.transmitted.x);
        if (dot(transmitted, transmitted) > 0)
        {
            reflection = false;
            refracted = incident.x != material.transmitted.x;
            return normalize(transmitted);
        }
    }
    return reflect(direction, normal);
}

func trace_guide_impl<Scene>(scene: Scene, first: TransportCall, budget: uint,
                             first_reflection: bool)
    ->GuideEndpoint
    where Scene : IGuideScene
{
    var result : GuideEndpoint = {};
    result.hit = first.hit;
    result.incident = first.path.medium;
    result.direction = first.path.direction;
    result.camera_origin = first.path.origin;
    result.camera_direction = first.path.direction;
    result.psr = psr_initial();
    result.queries = 1;
    result.material = scene.guide_material(result.hit, result.incident);
    result.pose = scene.guide_pose(result.hit);
    var candidate : GuideEndpoint = result;
    while (true)
    {
        let discrete = openpbr::is_discrete_only(result.material.pbr, result.incident,
                                                 result.material.transmitted);
        if (!discrete && !(first_reflection && result.queries == 1))
        {
            var position : float3;
            var normal : float3;
            if (psr_surface(result.psr, result.camera_origin, result.camera_direction,
                            result.hit.surface.position, result.hit.surface.normal,
                            shading_normal(result.hit, result.material), position, normal))
                return result;
            candidate.flags |= GUIDE_SURFACE_PROXY;
            return candidate;
        }
        if (result.queries == budget)
        {
            candidate.flags |= GUIDE_SURFACE_PROXY | GUIDE_BUDGET_PROXY;
            return candidate;
        }
        var reflection : bool;
        var refracted : bool;
        var next : float3 = guide_delta_direction(
                       result.material, shading_normal(result.hit, result.material),
                       result.direction, result.incident, reflection, refracted);
        if (first_reflection && result.queries == 1)
        {
            next = reflect(result.direction, shading_normal(result.hit, result.material));
            reflection = true;
            refracted = false;
        }
        if (result.queries == 1 && !reflection && !first_reflection)
        {
            let interface_guides = denoise_surface(scene.energy(), result.material,
                                                   shading_normal(result.hit, result.material),
                                                   -result.direction, result.incident);
            result.interface_specular = interface_guides.specular_albedo;
            result.use_interface_specular = true;
        }
        result.psr = psr_append(result.psr, result.hit.surface.position,
                                shading_normal(result.hit, result.material), reflection, refracted,
                                result.pose.fixed);
        if (refracted)
            result.flags |= GUIDE_REFRACTION_PROXY;
        let origin = geometry::offset_origin(result.hit.surface, next);
        if (!reflection && (result.material.pbr.control.x & 2u) == 0)
            result.incident = result.material.transmitted;
        result.direction = next;
        ++result.queries;
        if (!scene.intersect(origin, next, result.hit))
        {
            result.flags |= GUIDE_SKY;
            if (result.psr.count != 0)
                result.flags |= GUIDE_MOTION_PROXY;
            return result;
        }
        result.distance += result.hit.distance;
        scene.resolve_hit(result.hit, result.incident);
        result.material = scene.guide_material(result.hit, result.incident);
        result.pose = scene.guide_pose(result.hit);
    }
}

/// 从共享首命中独立完成规范 guide；budget 为 1..64，包含共享首查询和逃逸查询。
/// 第 N 次得到非 delta 或 escape 仍完成；只在需要第 N+1 次时返回预算代理。
/// 首命中须 resolve_hit；忽略照明 status、beta、radiance、depth、dielectric_limit 与随机数。
/// 后缀串行迭代，不分配、不调度；最坏增加 budget-1 次查询，不改写 first 或照明。
public func trace_guide<Scene>(scene: Scene, first: TransportCall, budget: uint)->GuideEndpoint
    where Scene : IGuideScene
{
    return trace_guide_impl(scene, first, budget, false);
}

/// 首面强制独立理想反射，之后沿规范 delta 链；用于显式反射运动代理。
/// 粗糙表面的理想反射是局部几何代理，不等于随机照明路径或真实粗反射光流。
/// 预算、查询与所有权约定同 trace_guide；budget=1 仅返回首接口预算代理。
public func trace_reflection_guide<Scene>(scene: Scene, first: TransportCall, budget: uint)
    ->GuideEndpoint
    where Scene : IGuideScene
{
    return trace_guide_impl(scene, first, budget, true);
}

/// 终点方向能量，不乘路径吞吐/照明/曝光；首透明面保留接口 specular，diffuse 取终点。
/// 仅用于非 SKY/INVALID 的表面；法线仍在实际终点空间，PSR 法线由 project_guide 返回。
public func guide_albedos<Energy>(energy: Energy, endpoint: GuideEndpoint)->DenoiseGuides
    where Energy : openpbr::IEnergyLut
{
    var guides : DenoiseGuides = denoise_surface(energy, endpoint.material,
                                                 shading_normal(endpoint.hit, endpoint.material),
                                                 -endpoint.direction, endpoint.incident);
    if (endpoint.use_interface_specular)
        guides.specular_albedo = endpoint.interface_specular;
    return guides;
}

/// 一致的几何输出与显式代理标记；motion 为输入像素 previous-current，不含 jitter。
public struct GuideProjection
{
    public var depth : float;
    public var normal : float3;
    public var motion : float2;
    public var flags : uint;
};

/// 将规范终点投影为相机 guide；current/previous 使用同一锚点，sample_uv 含实际 jitter。
/// history_valid=false 时运动为零；未知姿态使用相机代理，并保留标记。
/// 非法或 RG16F 不可表示的运动写有限零且标代理，不向 SDK 传内部哨兵。
public func project_guide(endpoint: GuideEndpoint, current: DenoiseCamera, previous: DenoiseCamera,
                          sample_uv: float2, extent: uint2, history_valid: bool)
    ->GuideProjection
{
    var result : GuideProjection = { DENOISE_SKY_DEPTH, float3(0), float2(0), endpoint.flags };
    var target : float3 = endpoint.camera_direction;
    let sky : bool = (endpoint.flags & GUIDE_SKY) != 0;
    if (!sky)
    {
        var position : float3;
        var normal : float3;
        if (!psr_surface(endpoint.psr, endpoint.camera_origin, endpoint.camera_direction,
                         endpoint.hit.surface.position, endpoint.hit.surface.normal,
                         shading_normal(endpoint.hit, endpoint.material), position, normal))
        {
            result.flags |= GUIDE_INVALID;
            return result;
        }
        result.depth = denoise_depth(current, position);
        result.normal = dot(normal, -endpoint.camera_direction) < 0 ? -normal : normal;
        if (!(result.depth > 0) || !isfinite(result.depth))
        {
            result.depth = 0;
            result.flags |= GUIDE_INVALID;
            return result;
        }
        let known = endpoint.pose.known && endpoint.psr.fixed;
        target = position;
        if (known)
            target += psr_direction(endpoint.psr, endpoint.pose.previous_position -
                                                      endpoint.hit.surface.position);
        else
            result.flags |= GUIDE_MOTION_PROXY;
    }
    if (history_valid &&
        (!denoise_motion(previous, sample_uv, target, sky, extent, result.motion) ||
         any(abs(result.motion) > 65504)))
    {
        result.motion = 0;
        result.flags |= GUIDE_MOTION_PROXY;
    }
    return result;
}

/// 未抖动相机的局部反射距离代理；distance=0 复用实际 primary motion，天空无平移视差。
/// 非零距离不能补齐反射目标的对象前态；false 时返回有限零，由上层选择主 motion 退路。
public func reflection_distance_motion(current: DenoiseCamera, previous: DenoiseCamera,
                                       sample_uv: float2, depth: float, distance: float,
                                       primary_motion: float2, extent: uint2, out motion: float2)
    ->bool
{

    motion = 0;
    let direction = denoise_direction(current, sample_uv);
    if (depth == DENOISE_SKY_DEPTH)
        return denoise_motion(previous, sample_uv, direction, true, extent, motion);
    if (!(depth > 0) || !isfinite(depth) || !isfinite(distance) || distance < 0)
        return false;
    if (distance == 0)
    {
        if (!all(isfinite(primary_motion)))
            return false;
        motion = primary_motion;
        return true;
    }
    let cosine = dot(direction, current.forward.xyz);
    if (!(cosine > 0))
        return false;
    let point : float3 = current.position.xyz + direction * (depth / cosine + distance);
    return denoise_motion(previous, sample_uv, point, false, extent, motion);
}

/// 同一稳定刚体实例/有序三角形的前帧位置；previous 为 object-to-world 3x4。
/// 调用方证明身份与重心对应，安全射线偏移不参与物理位置重建。
public func rigid_previous_position(a: float3, b: float3, c: float3, bary: float2,
                                    previous: float3x4)
    ->float3
{
    precise let local : float3 = a + mad(bary.x, b - a, bary.y * (c - a));
    precise var position : float3;
    for (var row = 0; row < 3; ++row)
        position[row] =
            previous[row][3] + mad(previous[row][0], local.x,
                                   mad(previous[row][1], local.y, previous[row][2] * local.z));
    return position;
}
}
