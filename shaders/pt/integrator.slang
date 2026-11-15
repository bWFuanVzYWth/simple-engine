/// 公共顶点输运：Beer、发光、双策略 NEE／MIS、OpenPBR、介质更新和 RR。
module integrator;

__exported import pt.scene;
__exported import pt.path_sampler;
import pt.transport;
import pt.roulette;
import geometry.ray;
import bsdf.openpbr.api;
import bsdf.openpbr.specialized;
import bsdf.openpbr.flow;
import bsdf.openpbr.transmission_composition;
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/contract/material.slang";

namespace pt
{
public func finite3(value: float3)->bool
{
    return all(isfinite(value));
}

public func finish(inout path: PathState, status: uint)->void
{
    path.depth += 1;
    path.status = finite3(path.throughput) && finite3(path.radiance) ? status : PATH_INVALID;
    let active : bool = path.status == PATH_ACTIVE;
    path.origin = select(active, path.origin, float3(0));
    path.previous = select(active, path.previous, float3(0));
    path.previous_pdf = select(active, path.previous_pdf, 0.0);
}

func accumulate_emission<Scene>(scene: Scene, hit: Hit, emission: float3, inout path: PathState)
    ->void
    where Scene : IScene
{
    if (any(emission > 0))
    {
        let weight : float = path.previous_pdf > 0
                                 ? power_heuristic(path.previous_pdf,
                                                   scene.emission_pdf(hit, path.previous))
                                 : 1;
        path.radiance += path.throughput * emission * weight;
    }
}

/// 无发光的delta solid前缀；类型契约只提供界面法线、目标介质与透射滤色。
/// 不建立通用材质，不读能量表，不生成光源采样与连续BSDF逻辑。
public func process_delta_prefix<Scene>(scene: Scene, inout data: TransportCall, settings: Settings)
    ->void
    where Scene : IDeltaPrefixScene
{
    var path : PathState = {};
    path.direction = data.path.direction;
    path.throughput = data.path.throughput;
    path.radiance = data.path.radiance;
    path.medium = data.path.medium;
    path.eta_scale = data.path.eta_scale;
    path.depth = data.path.depth;
    path.dielectric_events = data.path.dielectric_events;
    let hit : Hit = data.hit;
    path.throughput *= segment_transmittance(path.medium.yzw, hit.distance);
    let view : float3 = -path.direction;
    let continuation : bool = path.depth < settings.budget;
    let sampler : PathSampler =
                      path_sampler(data.pixel, data.sample, settings.resolution, settings.seed);
    if (!continuation)
    {
        finish(path, PATH_DONE);
        data.path = path;
        return;
    }
    path.dielectric_events += 1;
    if (path.dielectric_events > settings.dielectric_limit)
    {
        finish(path, PATH_DIELECTRIC_LIMIT);
        data.path = path;
        return;
    }
    let boundary : DeltaInterface = scene.delta_interface(hit, path.medium);
    let frame : PrimeFullOpenPbrOnb =
                    openpbr::specialized_frame<openpbr::CAP_DELTA_SOLID_DIELECTRIC>(
                        boundary.normal, view, path.medium, boundary.transmitted);
    let ior : float = path.medium.x > boundary.transmitted.x
                          ? path.medium.x / boundary.transmitted.x
                          : boundary.transmitted.x / path.medium.x;
    let scattered
        : openpbr::Sample = openpbr::specialized_result<openpbr::CAP_DELTA_SOLID_DIELECTRIC, false>(
              openpbr_composition::SolidDelta.sample(primeFullOpenPbrOnbToLocal(frame, view),
                                                     sampler.bsdf(path.depth), ior),
              frame, view, path.medium, boundary.transmitted);
    if (scattered.events == 0)
    {
        finish(path, PATH_DONE);
        data.path = path;
        return;
    }
    var response : float3 = scattered.response;
    if (dot(view, hit.surface.normal) * dot(scattered.direction, hit.surface.normal) < 0)
        response *= boundary.filter;
    path.direction = scattered.direction;
    path.throughput = product_over(path.throughput, response, scattered.pdf);
    let transmission : bool = (scattered.events & openpbr::TRANSMISSION) != 0;
    path.eta_scale = eta_scale_after_scatter(path.eta_scale, scattered.relative_eta, transmission);
    path.medium = scattered.medium;
    path.previous_pdf = 0;
    path.previous = hit.surface.position;
    path.origin = geometry::offset_origin(hit.surface, path.direction);
    var status : uint = PATH_ACTIVE;
    if (max(path.throughput.x, max(path.throughput.y, path.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (path.depth + 1 >= 2)
    {
        let survival : float = roulette_probability(path.throughput, path.eta_scale,
                                                    settings.square_root_roulette);
        if (survival == 0)
            status = PATH_DONE;
        else if (survival < 1.0)
        {
            if (sampler.roulette(path.depth) >= survival)
                status = PATH_DONE;
            else
                path.throughput = roulette_reweight(path.throughput, survival);
        }
    }
    finish(path, status);
    data.path = path;
}

func visible_flow<let Flow : uint, Scene, let Deferred : bool = false>(scene: Scene, hit: Hit,
                                                                       material: SurfaceMaterial,
                                                                       view: float3,
                                                                       light: LightSample,
                                                                       medium: float4)
    ->bool
    where Scene : IFlowScene
{
    return light.pdf > 0 &&
           openpbr::may_evaluate_flow<Flow>(material.pbr, shading_normal(hit, material), view,
                                            light.direction, medium, material.transmitted) &&
           (Deferred || !scene.occluded(light.visibility));
}

/// 不含光源辐射的两族贡献；材质阶段已折叠BSDF、MIS、介质和路径吞吐量。
/// raygen按相同采样身份重建光样本并查询可见性后消费，不持久化射线或材质闭包。
public struct DirectWeights
{
    public var sun : float3;
    public var lamp : float3;
};

func direct_weight<let Flow : uint>(point_value: geometry::SurfacePoint, view: float3,
                                    incident: float4, transmitted: float4, light: LightSample,
                                    evaluated: float4, throughput: float3)
    ->float3
{
    var extinction : float3 = incident.yzw;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
        dot(view, point_value.normal) * dot(light.direction, point_value.normal) < 0)
        extinction = transmitted.yzw;
    return throughput * triple_product(evaluated.xyz,
                                       segment_transmittance(extinction, light.distance),
                                       evaluated.w);
}

func direct_evaluation(evaluated: openpbr::Evaluation, light_pdf: float, continuation: bool)->float4
{
    if (light_pdf <= 0 || !any(evaluated.response > 0))
        return 0;
    return float4(evaluated.response,
                  power_heuristic_over_pdf(light_pdf, continuation ? evaluated.pdf : 0));
}

func direct_contribution<let Flow : uint, Scene>(scene: Scene, point_value: geometry::SurfacePoint,
                                                 view: float3, incident: float4,
                                                 transmitted: float4, light: LightSample,
                                                 evaluated: float4, solar: bool, throughput: float3)
    ->float3
    where Scene : IFlowScene
{
    var extinction : float3 = incident.yzw;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
        dot(view, point_value.normal) * dot(light.direction, point_value.normal) < 0)
        extinction = transmitted.yzw;
    let t : float3 = segment_transmittance(extinction, light.distance);
    let emission : float3 = scene.light_emission(point_value, light, solar);
    return throughput * triple_product(evaluated.xyz, emission * t, evaluated.w);
}

// 太阳只折叠 MIS，面灯立即形成最终贡献；大气求值始终在共享 BSDF 状态结束之后。
struct FlowLighting<let Flow : uint, Scene, let Deferred : bool = false>
    : openpbr::IFlowEvaluationConsumer
    where Scene : IFlowScene
{
    var scene : Scene;
    var point : geometry::SurfacePoint;
    var view : float3;
    var incident : float4;
    var transmitted : float4;
    var sun : LightSample;
    var light : LightSample;
    var throughput : float3;
    var continuation : bool;
    var sun_evaluated : float4;
    var lamp_radiance : float3;
    var lamp_valid : bool;
    var lamp_weight : float3;

    [mutating]
    func consume(direction: uint, evaluated: openpbr::Evaluation)->void
    {
        if (direction == 0u)
            sun_evaluated = direct_evaluation(evaluated, sun.pdf, continuation);
        else
        {
            let response : float4 = direct_evaluation(evaluated, light.pdf, continuation);
            if (any(response.xyz > 0))
            {
                if (Deferred)
                    lamp_weight = direct_weight<Flow>(point, view, incident, transmitted, light,
                                                      response, throughput);
                else
                    lamp_radiance =
                        direct_contribution<Flow>(scene, point, view, incident, transmitted, light,
                                                  response, false, throughput);
                lamp_valid = true;
            }
        }
    }
};

/// 一次 callable 消费完整组合顶点；分量不再嵌套调用 callable。
/// Flow 由资源世代证明，运行时仅翻译参数、查询可见性和采样实际分量。
/// CompleteTransport=false由调用方先消费命中段Beer，返回后执行统一RR和finish；
/// 此时depth不递增，status为材质顶点结果，不能直接送入下一次追踪。
public func process_flow<let Flow : uint, Scene, let LightMask : uint = 3,
                         let CompleteTransport : bool = true>(scene: Scene,
                                                              inout data: TransportCall,
                                                              settings: Settings)
    ->void
    where Scene : IFlowScene
{
    var unused : DirectWeights;
    process_flow_vertex<Flow, Scene, LightMask, CompleteTransport, false>(scene, data, settings,
                                                                          unused);
}

/// Deferred=true时不在材质叶子中追踪阴影或读取光源辐射；调用者消费weights。
public func process_flow_vertex<let Flow : uint, Scene, let LightMask : uint = 3,
                                let CompleteTransport : bool = true, let Deferred : bool = true>(
    scene: Scene, inout data: TransportCall, settings: Settings, out weights: DirectWeights)
    ->void
    where Scene : IFlowScene
{
    weights = {};
    static_assert((LightMask & ~3u) == 0u, "unknown light strategy mask");
    var path : PathState = {};
    path.direction = data.path.direction;
    path.throughput = data.path.throughput;
    path.radiance = data.path.radiance;
    path.medium = data.path.medium;
    path.eta_scale = data.path.eta_scale;
    path.depth = data.path.depth;
    path.dielectric_events = data.path.dielectric_events;
    if ((Flow & openpbr::FLOW_EMISSION) != 0u)
    {
        path.previous = data.path.previous;
        path.previous_pdf = data.path.previous_pdf;
    }
    let hit : Hit = data.hit;
    if (CompleteTransport)
        path.throughput *= segment_transmittance(path.medium.yzw, hit.distance);
    let material : SurfaceMaterial = scene.resolve_flow(hit, Flow, path.medium);
    if ((Flow & openpbr::FLOW_EMISSION) != 0u)
        accumulate_emission(scene, hit, material.emission, path);
    var view : float3 = -path.direction;
    var continuation : bool = path.depth < settings.budget;
    let sampler : PathSampler =
                      path_sampler(data.pixel, data.sample, settings.resolution, settings.seed);
    var sun : LightSample = {};
    var light : LightSample = {};
    var evaluation_mask : uint = 0;
    if (((Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                  openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) !=
         0u))
    {
        if ((LightMask & 1u) != 0u)
        {
            sun = scene.sample_sun(hit.surface, sampler.sun(path.depth));
            if (visible_flow<Flow, Scene, Deferred>(scene, hit, material, view, sun, path.medium))
                evaluation_mask |= 1u;
        }
        if ((LightMask & 2u) != 0u)
        {
            light = scene.sample_light(hit.surface, sampler.light(path.depth));
            if (visible_flow<Flow, Scene, Deferred>(scene, hit, material, view, light, path.medium))
                evaluation_mask |= 2u;
        }
    }
    let limited : bool = (Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
                         path.dielectric_events + 1 > settings.dielectric_limit;
    var bsdf_sample = float3(0.0);
    if ((Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                 openpbr::FLOW_DIELECTRIC_REFLECTION | openpbr::FLOW_CONDUCTOR_REFLECTION |
                 openpbr::FLOW_SOLID_TRANSMISSION | openpbr::FLOW_THIN_TRANSMISSION)) != 0u &&
        continuation && !limited)
        bsdf_sample = sampler.bsdf(path.depth);
    var lighting : FlowLighting<Flow, Scene, Deferred> = {};
    lighting.scene = scene;
    lighting.point = hit.surface;
    lighting.view = view;
    lighting.incident = path.medium;
    lighting.transmitted = material.transmitted;
    lighting.sun = sun;
    lighting.light = light;
    lighting.throughput = path.throughput;
    lighting.continuation = continuation;
    let scattered : openpbr::Sample =
                        openpbr::shade_flow_consume<Flow, openpbr::Energy,
                                                    FlowLighting<Flow, Scene, Deferred>, LightMask>(
                            scene.energy(), material.pbr, shading_normal(hit, material), view,
                            sun.direction, light.direction, evaluation_mask, bsdf_sample,
                            continuation && !limited, path.medium, material.transmitted, lighting);
    if (((Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                  openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) !=
         0u))
    {
        if ((LightMask & 1u) != 0u && any(lighting.sun_evaluated.xyz > 0))
        {
            if (Deferred)
                weights.sun =
                    direct_weight<Flow>(hit.surface, view, path.medium, material.transmitted, sun,
                                        lighting.sun_evaluated, path.throughput);
            else
                path.radiance += direct_contribution<Flow>(
                    scene, hit.surface, view, path.medium, material.transmitted, sun,
                    lighting.sun_evaluated, true, path.throughput);
        }
        if (Deferred)
            weights.lamp = lighting.lamp_weight;
        else if ((LightMask & 2u) != 0u && lighting.lamp_valid)
            path.radiance += lighting.lamp_radiance;
    }
    if (!continuation)
    {
        if (CompleteTransport)
            finish(path, PATH_DONE);
        else
            path.status = PATH_DONE;
        data.path = path;
        return;
    }
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u)
    {
        path.dielectric_events += 1;
        if (path.dielectric_events > settings.dielectric_limit)
        {
            if (CompleteTransport)
                finish(path, PATH_DIELECTRIC_LIMIT);
            else
                path.status = PATH_DIELECTRIC_LIMIT;
            data.path = path;
            return;
        }
    }
    if (scattered.events == 0)
    {
        if (CompleteTransport)
            finish(path, PATH_DONE);
        else
            path.status = PATH_DONE;
        data.path = path;
        return;
    }
    var response : float3 = scattered.response;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
        (scattered.events & openpbr::DISCRETE) != 0u &&
        dot(view, hit.surface.normal) * dot(scattered.direction, hit.surface.normal) < 0)
        response *= material.boundary_filter;
    path.direction = scattered.direction;
    path.throughput = product_over(path.throughput, response, scattered.pdf);
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u)
    {
        let transmission : bool = (scattered.events & openpbr::TRANSMISSION) != 0;
        path.eta_scale =
            eta_scale_after_scatter(path.eta_scale, scattered.relative_eta, transmission);
        path.medium = scattered.medium;
    }
    path.previous_pdf =
        (Flow & (openpbr::FLOW_DELTA_REFLECTION | openpbr::FLOW_DELTA_TRANSMISSION)) == 0u
            ? scattered.pdf
        : (scattered.events & openpbr::DISCRETE) != 0u ? 0.0
                                                       : scattered.pdf;
    path.previous = hit.surface.position;
    path.origin = geometry::offset_origin(hit.surface, path.direction);
    var status : uint = PATH_ACTIVE;
    if (CompleteTransport && max(path.throughput.x, max(path.throughput.y, path.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (CompleteTransport && path.depth + 1 >= 2)
    {
        let survival : float = roulette_probability(path.throughput, path.eta_scale,
                                                    settings.square_root_roulette);
        if (survival == 0)
            status = PATH_DONE;
        else if (survival < 1.0)
        {
            if (sampler.roulette(path.depth) >= survival)
                status = PATH_DONE;
            else
                path.throughput = roulette_reweight(path.throughput, survival);
        }
    }
    if (CompleteTransport)
        finish(path, status);
    else
        path.status = status;
    data.path = path;
}

/// 未命中时消费末段 Beer 与环境分项；独立太阳策略按上一连续 PDF 作 MIS。
public func escape<Scene>(scene: Scene, inout path: PathState)->void
    where Scene : IScene
{
    path.throughput *= segment_transmittance(path.medium.yzw, FAR);
    let environment : Environment = scene.environment(path.direction, path.previous, path.depth);
    let weight : float = path.previous_pdf > 0 && environment.sun_pdf > 0
                             ? power_heuristic(path.previous_pdf, environment.sun_pdf)
                             : 1;
    path.radiance += path.throughput * (environment.sky + environment.sun * weight);
    path.status = finite3(path.radiance) ? PATH_DONE : PATH_INVALID;
    path.origin = 0;
    path.previous = 0;
    path.previous_pdf = 0;
}

/// K1 只内联纯 delta solid 前缀；首个其他能力表面交给 K2，不增加材质缓存。
public func trace_prefix<Scene>(scene: Scene, inout data: TransportCall, settings: Settings,
                                out first_distance: float)
    ->void
    where Scene : ITraceScene
{
    trace_prefix(scene, data, settings, first_distance, NoPrefixGuides());
}

/// 可选首命中辅助量观察者；关闭时与无观察者入口相同，delta 前缀仍使用原有随机路径。
public func trace_prefix<Scene, Guides>(scene: Scene, inout data: TransportCall, settings: Settings,
                                        out first_distance: float, guides: Guides)
    ->void
    where Scene : ITraceScene
    where Guides : IPrefixGuides
{
    first_distance = FAR;
    while (data.path.status == PATH_ACTIVE)
    {
        if (!scene.intersect(data.path.origin, data.path.direction, data.hit))
        {
            if (data.path.depth == 0)
                guides.environment(data);
            escape(scene, data.path);
            break;
        }
        if (data.path.depth == 0)
            first_distance = data.hit.distance;
        scene.resolve_hit(data.hit, data.path.medium);
        if (data.path.depth == 0)
            guides.surface(data);
        if (!scene.prefix(data.hit))
            break;
        process_delta_prefix(scene, data, settings);
    }
}

/// K2 的唯一循环；TraceRay 和一次叶子 CallShader 由编译期场景适配器提供。
public func trace_tail<Scene>(scene: Scene, inout data: TransportCall)->void
    where Scene : ITraceScene
{
    var scene = scene;
    while (true)
    {
        scene.shade(data);
        if (data.path.status != PATH_ACTIVE)
            break;
        if (!scene.intersect(data.path.origin, data.path.direction, data.hit))
        {
            escape(scene, data.path);
            break;
        }
        scene.resolve_hit(data.hit, data.path.medium);
    }
}
}
