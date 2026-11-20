/// 公共顶点输运：Beer、发光、双策略 NEE／MIS、OpenPBR、介质更新和 RR。
module integrator;

__exported import pt.scene;
__exported import pt.path_sampler;
import transport.roulette;
import geometry.ray;
import openpbr.api;
import openpbr.specialized;
import openpbr.flow;
import openpbr.transmission_composition;
import "openpbr/math/frame.slang";
import "openpbr/contract/material.slang";
import light_sampling.sample;
import transport.medium;
import transport.surface_material;
import transport.weights;

namespace pt {
public func finish(inout path: PathState, status: uint)->void {
    path.depth += 1;
    path.status =
        all(isfinite(path.throughput)) && all(isfinite(path.radiance)) ? status : PATH_INVALID;
    let active = path.status == PATH_ACTIVE;
    path.origin = select(active, path.origin, float3(0));
    path.previous = select(active, path.previous, float3(0));
    path.previous_pdf = select(active, path.previous_pdf, 0.0);
}

func accumulate_emission<Scene>(scene: Scene, hit: Hit, emission: float3, inout path: PathState)
    ->void
    where Scene : IScene {
    if (any(emission > 0)) {
        let weight : float = path.previous_pdf > 0
                                 ? transport::power_heuristic(
                                       path.previous_pdf, scene.emission_pdf(hit, path.previous))
                                 : 1;
        path.radiance += path.throughput * emission * weight;
    }
}

/// 无发光的delta solid前缀；类型契约只提供界面法线、目标介质与透射滤色。
/// 不建立通用材质，不读能量表，不生成光源采样与连续BSDF逻辑。
public func process_delta_prefix<Scene>(scene: Scene, inout data: TransportCall, settings: Settings)
    ->void
    where Scene : IDeltaPrefixScene {
    var path : PathState = {};
    path.direction = data.path.direction;
    path.throughput = data.path.throughput;
    path.radiance = data.path.radiance;
    path.medium = data.path.medium;
    path.eta_scale = data.path.eta_scale;
    path.depth = data.path.depth;
    path.dielectric_events = data.path.dielectric_events;
    let hit : Hit = data.hit;
    path.throughput *= transport::segment_transmittance(path.medium.yzw, hit.distance);
    let view = -path.direction;
    let continuation = path.depth < settings.budget;
    let sampler = path_sampler(data.pixel, data.sample, settings.resolution, settings.seed);
    if (!continuation) {
        finish(path, PATH_DONE);
        data.path = path;
        return;
    }
    path.dielectric_events += 1;
    if (path.dielectric_events > settings.dielectric_limit) {
        finish(path, PATH_DIELECTRIC_LIMIT);
        data.path = path;
        return;
    }
    let boundary : DeltaInterface = scene.delta_interface(hit, path.medium);
    let frame : openpbr::kernel::Frame =
                    openpbr::specialized_frame<openpbr::CAP_DELTA_SOLID_DIELECTRIC>(
                        boundary.normal, view, path.medium, boundary.transmitted);
    let ior = path.medium.x > boundary.transmitted.x ? path.medium.x / boundary.transmitted.x
                                                     : boundary.transmitted.x / path.medium.x;
    let scattered
        : openpbr::Sample = openpbr::specialized_result<openpbr::CAP_DELTA_SOLID_DIELECTRIC, false>(
              openpbr::closure::SolidDelta.sample(openpbr::kernel::to_local(frame, view),
                                                  sampler.bsdf(path.depth), ior),
              frame, view, path.medium, boundary.transmitted);
    if (scattered.events == 0) {
        finish(path, PATH_DONE);
        data.path = path;
        return;
    }
    var response : float3 = scattered.response;
    if (dot(view, hit.surface.normal) * dot(scattered.direction, hit.surface.normal) < 0)
        response *= boundary.filter;
    path.direction = scattered.direction;
    path.throughput = transport::product_over(path.throughput, response, scattered.pdf);
    let transmission : bool = (scattered.events & openpbr::TRANSMISSION) != 0;
    path.eta_scale =
        transport::eta_scale_after_scatter(path.eta_scale, scattered.relative_eta, transmission);
    path.medium = scattered.medium;
    path.previous_pdf = 0;
    path.previous = hit.surface.position;
    path.origin = geometry::offset_origin(hit.surface, path.direction);
    var status : uint = PATH_ACTIVE;
    if (max(path.throughput.x, max(path.throughput.y, path.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (path.depth + 1 >= 2) {
        let survival = transport::roulette_probability(path.throughput, path.eta_scale);
        if (survival == 0)
            status = PATH_DONE;
        else if (survival < 1.0) {
            if (sampler.roulette(path.depth) >= survival)
                status = PATH_DONE;
            else
                path.throughput = transport::roulette_reweight(path.throughput, survival);
        }
    }
    finish(path, status);
    data.path = path;
}

/// 单侧反射只能连接几何表面同侧的光源；法线贴图只改变该半球内的响应。
public func light_side_supported<let flow : uint>(hit: Hit, material: transport::SurfaceMaterial,
                                                  view: float3, direction: float3)
    ->bool {
    return (flow & (openpbr::FLOW_SOLID_TRANSMISSION | openpbr::FLOW_THIN_TRANSMISSION)) != 0 ||
           ((flow & openpbr::FLOW_SUBSURFACE) != 0 && material.pbr.surface.z > 0) ||
           dot(view, hit.surface.normal) * dot(direction, hit.surface.normal) > 0;
}

func visible_flow<let flow : uint, Scene, let deferred : bool = false>(
    scene: Scene, hit: Hit, material: transport::SurfaceMaterial, view: float3,
    light: light_sampling::LightSample, medium: float4)
    ->bool
    where Scene : IFlowScene {
    return light.pdf > 0 && light_side_supported<flow>(hit, material, view, light.direction) &&
           openpbr::may_evaluate_flow<flow>(material.pbr, shading_normal(hit, material), view,
                                            light.direction, medium, material.transmitted) &&
           (deferred || !scene.occluded(light.visibility));
}

/// 不含光源辐射的两族贡献；材质阶段已折叠BSDF、MIS、介质和路径吞吐量。
/// raygen按相同采样身份重建光样本并查询可见性后消费，不持久化射线或材质闭包。
public struct DirectWeights {
    public var sun : float3;
    public var lamp : float3;
};

func direct_weight<let flow : uint>(point_value: geometry::SurfacePoint, view: float3,
                                    incident: float4, transmitted: float4,
                                    light: light_sampling::LightSample, evaluated: float4,
                                    throughput: float3)
    ->float3 {
    var extinction : float3 = incident.yzw;
    if ((flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
        dot(view, point_value.normal) * dot(light.direction, point_value.normal) < 0)
        extinction = transmitted.yzw;
    return throughput *
           transport::triple_product(evaluated.xyz,
                                     transport::segment_transmittance(extinction, light.distance),
                                     evaluated.w);
}

func direct_evaluation(evaluated: openpbr::Evaluation, light_pdf: float, continuation: bool)
    ->float4 {
    if (light_pdf <= 0 || !any(evaluated.response > 0))
        return 0;
    return float4(evaluated.response,
                  transport::power_heuristic_over_pdf(light_pdf, continuation ? evaluated.pdf : 0));
}

func direct_contribution<let flow : uint, Scene>(scene: Scene, point_value: geometry::SurfacePoint,
                                                 view: float3, incident: float4,
                                                 transmitted: float4,
                                                 light: light_sampling::LightSample,
                                                 evaluated: float4, solar: bool, throughput: float3)
    ->float3
    where Scene : IFlowScene {
    var extinction : float3 = incident.yzw;
    if ((flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
        dot(view, point_value.normal) * dot(light.direction, point_value.normal) < 0)
        extinction = transmitted.yzw;
    let t = transport::segment_transmittance(extinction, light.distance);
    let emission = scene.light_emission(point_value, light, solar);
    return throughput * transport::triple_product(evaluated.xyz, emission * t, evaluated.w);
}

// 太阳只折叠 MIS，面灯立即形成最终贡献；大气求值始终在共享 BSDF 状态结束之后。
struct FlowLighting<let flow : uint, Scene, let deferred : bool = false>
    : openpbr::IFlowEvaluationConsumer
    where Scene : IFlowScene {
    var scene : Scene;
    var point : geometry::SurfacePoint;
    var view : float3;
    var incident : float4;
    var transmitted : float4;
    var sun : light_sampling::LightSample;
    var light : light_sampling::LightSample;
    var throughput : float3;
    var continuation : bool;
    var sun_evaluated : float4;
    var lamp_radiance : float3;
    var lamp_valid : bool;
    var lamp_weight : float3;

    [mutating]
    func consume(direction: uint, evaluated: openpbr::Evaluation)->void {
        if (direction == 0u)
            sun_evaluated = direct_evaluation(evaluated, sun.pdf, continuation);
        else {
            let response = direct_evaluation(evaluated, light.pdf, continuation);
            if (any(response.xyz > 0)) {
                if (deferred)
                    lamp_weight = direct_weight<flow>(point, view, incident, transmitted, light,
                                                      response, throughput);
                else
                    lamp_radiance =
                        direct_contribution<flow>(scene, point, view, incident, transmitted, light,
                                                  response, false, throughput);
                lamp_valid = true;
            }
        }
    }
};

/// 一次 callable 消费完整组合顶点；分量不再嵌套调用 callable。
/// flow 由资源世代证明，运行时仅翻译参数、查询可见性和采样实际分量。
/// complete_transport=false由调用方先消费命中段Beer，返回后执行统一RR和finish；
/// 此时depth不递增，status为材质顶点结果，不能直接送入下一次追踪。
public func process_flow<let flow : uint, Scene, let light_mask : uint = 3,
                         let complete_transport : bool = true>(scene: Scene,
                                                               inout data: TransportCall,
                                                               settings: Settings)
    ->void
    where Scene : IFlowScene {
    var unused : DirectWeights;
    process_flow_vertex<flow, Scene, light_mask, complete_transport, false>(scene, data, settings,
                                                                            unused);
}

/// deferred=true时不在材质叶子中追踪阴影或读取光源辐射；调用者消费weights。
public func process_flow_vertex<let flow : uint, Scene, let light_mask : uint = 3,
                                let complete_transport : bool = true, let deferred : bool = true>(
    scene: Scene, inout data: TransportCall, settings: Settings, out weights: DirectWeights)
    ->void
    where Scene : IFlowScene {
    weights = {};
    static_assert((light_mask & ~3u) == 0u, "unknown light strategy mask");
    var path : PathState = {};
    path.direction = data.path.direction;
    path.throughput = data.path.throughput;
    path.radiance = data.path.radiance;
    path.medium = data.path.medium;
    path.eta_scale = data.path.eta_scale;
    path.depth = data.path.depth;
    path.dielectric_events = data.path.dielectric_events;
    if ((flow & openpbr::FLOW_EMISSION) != 0u) {
        path.previous = data.path.previous;
        path.previous_pdf = data.path.previous_pdf;
    }
    let hit : Hit = data.hit;
    if (complete_transport)
        path.throughput *= transport::segment_transmittance(path.medium.yzw, hit.distance);
    let material : transport::SurfaceMaterial = scene.resolve_flow(hit, flow, path.medium);
    if ((flow & openpbr::FLOW_EMISSION) != 0u)
        accumulate_emission(scene, hit, material.emission, path);
    var view : float3 = -path.direction;
    var continuation : bool = path.depth < settings.budget;
    let sampler = path_sampler(data.pixel, data.sample, settings.resolution, settings.seed);
    var sun : light_sampling::LightSample = {};
    var light : light_sampling::LightSample = {};
    var evaluation_mask : uint = 0;
    if (((flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                  openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) !=
         0u)) {
        if ((light_mask & 1u) != 0u) {
            sun = scene.sample_sun(hit.surface, sampler.sun(path.depth));
            if (visible_flow<flow, Scene, deferred>(scene, hit, material, view, sun, path.medium))
                evaluation_mask |= 1u;
        }
        if ((light_mask & 2u) != 0u) {
            light = scene.sample_light(hit.surface, sampler.light(path.depth));
            if (visible_flow<flow, Scene, deferred>(scene, hit, material, view, light, path.medium))
                evaluation_mask |= 2u;
        }
    }
    let limited : bool = (flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
                         path.dielectric_events + 1 > settings.dielectric_limit;
    var bsdf_sample = float3(0.0);
    if ((flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                 openpbr::FLOW_DIELECTRIC_REFLECTION | openpbr::FLOW_CONDUCTOR_REFLECTION |
                 openpbr::FLOW_SOLID_TRANSMISSION | openpbr::FLOW_THIN_TRANSMISSION)) != 0u &&
        continuation && !limited)
        bsdf_sample = sampler.bsdf(path.depth);
    var lighting : FlowLighting<flow, Scene, deferred> = {};
    lighting.scene = scene;
    lighting.point = hit.surface;
    lighting.view = view;
    lighting.incident = path.medium;
    lighting.transmitted = material.transmitted;
    lighting.sun = sun;
    lighting.light = light;
    lighting.throughput = path.throughput;
    lighting.continuation = continuation;
    let scattered
        : openpbr::Sample =
              openpbr::shade_flow_consume<flow, openpbr::Energy,
                                          FlowLighting<flow, Scene, deferred>, light_mask>(
                  scene.energy(), material.pbr, shading_normal(hit, material), view, sun.direction,
                  light.direction, evaluation_mask, bsdf_sample, continuation && !limited,
                  path.medium, material.transmitted, lighting);
    if (((flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                  openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) !=
         0u)) {
        if ((light_mask & 1u) != 0u && any(lighting.sun_evaluated.xyz > 0)) {
            if (deferred)
                weights.sun =
                    direct_weight<flow>(hit.surface, view, path.medium, material.transmitted, sun,
                                        lighting.sun_evaluated, path.throughput);
            else
                path.radiance += direct_contribution<flow>(
                    scene, hit.surface, view, path.medium, material.transmitted, sun,
                    lighting.sun_evaluated, true, path.throughput);
        }
        if (deferred)
            weights.lamp = lighting.lamp_weight;
        else if ((light_mask & 2u) != 0u && lighting.lamp_valid)
            path.radiance += lighting.lamp_radiance;
    }
    if (!continuation) {
        if (complete_transport)
            finish(path, PATH_DONE);
        else
            path.status = PATH_DONE;
        data.path = path;
        return;
    }
    if ((flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u) {
        path.dielectric_events += 1;
        if (path.dielectric_events > settings.dielectric_limit) {
            if (complete_transport)
                finish(path, PATH_DIELECTRIC_LIMIT);
            else
                path.status = PATH_DIELECTRIC_LIMIT;
            data.path = path;
            return;
        }
    }
    if (scattered.events == 0 || !scatter_side_supported(hit, material, view, scattered)) {
        if (complete_transport)
            finish(path, PATH_DONE);
        else
            path.status = PATH_DONE;
        data.path = path;
        return;
    }
    var response : float3 = scattered.response;
    if ((flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u &&
        (scattered.events & openpbr::DISCRETE) != 0u &&
        dot(view, hit.surface.normal) * dot(scattered.direction, hit.surface.normal) < 0)
        response *= material.boundary_filter;
    path.direction = scattered.direction;
    path.throughput = transport::product_over(path.throughput, response, scattered.pdf);
    if ((flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0u) {
        let transmission : bool = (scattered.events & openpbr::TRANSMISSION) != 0;
        path.eta_scale = transport::eta_scale_after_scatter(path.eta_scale, scattered.relative_eta,
                                                            transmission);
        path.medium = scattered.medium;
    }
    path.previous_pdf =
        (flow & (openpbr::FLOW_DELTA_REFLECTION | openpbr::FLOW_DELTA_TRANSMISSION)) == 0u
            ? scattered.pdf
        : (scattered.events & openpbr::DISCRETE) != 0u ? 0.0
                                                       : scattered.pdf;
    path.previous = hit.surface.position;
    path.origin = geometry::offset_origin(hit.surface, path.direction);
    var status : uint = PATH_ACTIVE;
    if (complete_transport &&
        max(path.throughput.x, max(path.throughput.y, path.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (complete_transport && path.depth + 1 >= 2) {
        let survival = transport::roulette_probability(path.throughput, path.eta_scale);
        if (survival == 0)
            status = PATH_DONE;
        else if (survival < 1.0) {
            if (sampler.roulette(path.depth) >= survival)
                status = PATH_DONE;
            else
                path.throughput = transport::roulette_reweight(path.throughput, survival);
        }
    }
    if (complete_transport)
        finish(path, status);
    else
        path.status = status;
    data.path = path;
}

/// 未命中时消费末段 Beer 与环境分项；独立太阳策略按上一连续 PDF 作 MIS。
public func escape<Scene>(scene: Scene, inout path: PathState)->void
    where Scene : IScene {
    path.throughput *= transport::segment_transmittance(path.medium.yzw, FAR);
    let environment : Environment = scene.environment(path.direction, path.previous, path.depth);
    let weight = path.previous_pdf > 0 && environment.sun_pdf > 0
                     ? transport::power_heuristic(path.previous_pdf, environment.sun_pdf)
                     : 1;
    path.radiance += path.throughput * (environment.sky + environment.sun * weight);
    path.status = all(isfinite(path.radiance)) ? PATH_DONE : PATH_INVALID;
    path.origin = 0;
    path.previous = 0;
    path.previous_pdf = 0;
}

/// K1 只内联纯 delta solid 前缀；首个其他能力表面交给 K2，不增加材质缓存。
public func trace_prefix<Scene>(scene: Scene, inout data: TransportCall, settings: Settings,
                                out first_distance: float)
    ->void
    where Scene : ITraceScene {
    trace_prefix(scene, data, settings, first_distance, NoPrefixGuides());
}

/// 可选首命中辅助量观察者；关闭时与无观察者入口相同，delta 前缀仍使用原有随机路径。
public func trace_prefix<Scene, Guides>(scene: Scene, inout data: TransportCall, settings: Settings,
                                        out first_distance: float, guides: Guides)
    ->void
    where Scene : ITraceScene
    where Guides : IPrefixGuides {
    first_distance = FAR;
    while (data.path.status == PATH_ACTIVE) {
        if (!scene.intersect(data.path.origin, data.path.direction, data.hit)) {
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
    where Scene : ITraceScene {
    var scene = scene;
    while (true) {
        scene.shade(data);
        if (data.path.status != PATH_ACTIVE)
            break;
        if (!scene.intersect(data.path.origin, data.path.direction, data.hit)) {
            escape(scene, data.path);
            break;
        }
        scene.resolve_hit(data.hit, data.path.medium);
    }
}
}
