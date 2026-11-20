/// 完整材质顶点的closest-hit边界；几何和材质只在已经特化的命中程序中解码。
module material_vertex;
import pt.scene;
import pt.integrator;
import pt.path_sampler;
import transport.roulette;
import openpbr.flow;
import geometry.ray;
import light_sampling.sample;
import transport.medium;
import transport.surface_material;
import transport.weights;

namespace pt {
/// 88B无损逻辑交接；输入样本号复用输出origin.x，像素直接取发射内建量，不搬运Hit、纹素、法线或灯样本。
/// 输入state为depth[0..6]/dielectric_events[7..13]；输出为status[0..1]/depth/events。
/// 同一次 Invoke 内先消费入射状态，再发布出射状态。
public struct MaterialCall {
    public var origin_pixel : float3;
    public var direction : float3;
    public var throughput : float3;
    public var radiance : float3;
    public var medium : float4;
    public var previous : float3;
    public var previous_pdf : float;
    public var eta_scale : float;
    public var state : uint;

    public static func input(data: TransportCall)->MaterialCall {
        return { float3(asfloat(data.sample), 0, 0),
                 float3(0),
                 data.path.throughput,
                 data.path.radiance,
                 data.path.medium,
                 data.path.previous,
                 data.path.previous_pdf,
                 data.path.eta_scale,
                 data.path.depth | (data.path.dielectric_events << 7u) };
    }
    /// 首面输入只有样本身份、分离通道和已完成路径的辐射和；其余字段仅作为输出。
    public static func primary_input(sample_value: uint, lane: uint, radiance: float3)
        ->MaterialCall {
        var call : MaterialCall;
        call.origin_pixel = float3(asfloat(sample_value), asfloat(lane), 0);
        call.radiance = radiance;
        call.previous_pdf = 0;
        return call;
    }
    /// 首面叶子建立相机路径的不变量，避免调度器为常量初态搬运完整路径。
    [mutating]
    public func initialize_primary(incident: float4)->void {
        direction = 0;
        throughput = 1;
        medium = incident;
        previous = 0;
        previous_pdf = 0;
        eta_scale = 1;
        state = 0;
    }
    [mutating]
    public func finish_vertex(depth: uint, events: uint, status: uint)->void {
        var status = status;
        if (!all(isfinite(throughput)) || !all(isfinite(radiance)))
            status = PATH_INVALID;
        state = status | ((depth + 1u) << 2u) | (events << 9u);
        let active = status == PATH_ACTIVE;
        origin_pixel = select(active, origin_pixel, float3(0));
        previous = select(active, previous, float3(0));
        previous_pdf = select(active, previous_pdf, 0.0);
    }
    public func result()->PathState {
        return { origin_pixel,
                 direction,
                 throughput,
                 radiance,
                 medium,
                 previous,
                 previous_pdf,
                 eta_scale,
                 (state >> 2u) & 127u,
                 (state >> 9u) & 127u,
                 state & 3u };
    }
};

// 阴影射线、BVH 和灯的临时身份仅活到当前策略的可见性与辐射求值。
struct VisibleLight {
    var direction : float3;
    var incident_radiance : float3;
    var pdf : float;
    var distance : float;
};

func visible_light<let flow : uint, let solar : bool, Scene : IFlowScene>(
    scene: Scene, hit: Hit, material: transport::SurfaceMaterial, view: float3, incident: float4,
    light: light_sampling::LightSample, test_visibility: bool = true)
    ->VisibleLight {
    var visible : VisibleLight = {};
    if (light.pdf <= 0 || !light_side_supported<flow>(hit, material, view, light.direction) ||
        !openpbr::may_evaluate_flow<flow>(material.pbr, shading_normal(hit, material), view,
                                          light.direction, incident, material.transmitted) ||
        (test_visibility && scene.occluded(light.visibility)))
        return visible;
    var extinction : float3 = incident.yzw;
    if ((flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0 &&
        dot(view, hit.surface.normal) * dot(light.direction, hit.surface.normal) < 0)
        extinction = material.transmitted.yzw;
    visible.incident_radiance = scene.light_emission(hit.surface, light, solar) *
                                transport::segment_transmittance(extinction, light.distance);
    visible.direction = light.direction;
    visible.pdf = light.pdf;
    visible.distance = light.distance;
    return visible;
}

// evaluation_mask 只请求 pdf>0 的方向；即时折叠到已有辐射，不保留 BSDF 结果。
public struct PrimaryLighting {
    public var sun : float3;
    public var sun_ray : geometry::Ray;
    public var local : float3;
    public var local_light : float4;
    public var emitted : float3;
    public var events : uint;
};

struct DirectEvaluationConsumer<let observe : bool> : openpbr::IFlowEvaluationConsumer {
    var first : float4;
    var second : float4;
    var throughput : float3;
    var radiance : float3;
    var continuation : bool;
    var direct_sun : float3;
    var direct_local : float3;

    [mutating]
    func consume(direction: uint, evaluated: openpbr::Evaluation)->void {
        let light = direction == 0 ? first : second;
        if (any(evaluated.response > 0)) {
            let term = throughput *
                       transport::triple_product(evaluated.response, light.xyz,
                                                 transport::power_heuristic_over_pdf(
                                                     light.w, continuation ? evaluated.pdf : 0));
            if (!observe || direction != 0)
                radiance += term;
            if (observe && direction == 0)
                direct_sun = term;
            if (observe && direction == 1)
                direct_local = term;
        }
    }
};

/// 已解码的本次命中由一个材质入口消费；路径结果直接原位发布。
/// primary 为 true 时，调用方保证 primary_lane 是固定双样本中的第 0 或 1 条路径。
public func shade_material_observed<let flow : uint, Scene : IFlowScene, let light_mask : uint,
                                    let primary : bool, let observe : bool>(
    scene: Scene, inout call: MaterialCall, hit: Hit, material: transport::SurfaceMaterial,
    incoming: float3, settings: Settings, primary_lane: uint, out signals: PrimaryLighting)
    ->void {
    signals = {};
    let depth = call.state & 127u;
    var events : uint = (call.state >> 7u) & 127u;
    let sample = asuint(call.origin_pixel.x);
    call.throughput *= transport::segment_transmittance(call.medium.yzw, hit.distance);
    if ((flow & openpbr::FLOW_EMISSION) != 0 && any(material.emission > 0)) {
        let w : float = call.previous_pdf > 0
                            ? transport::power_heuristic(call.previous_pdf,
                                                         scene.emission_pdf(hit, call.previous))
                            : 1;
        call.radiance += call.throughput * material.emission * w;
        if (observe)
            signals.emitted = call.throughput * material.emission * w;
    }
    let continuation = depth < settings.budget;
    let solid : bool = (flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0;
    if (solid && continuation)
        events++;
    let limited = solid && events > settings.dielectric_limit;
    let view = -incoming;
    let sampler = path_sampler(DispatchRaysIndex().xy, sample, settings.resolution, settings.seed);
    static const let continuous : bool = (flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                                                  openpbr::FLOW_CONTINUOUS_REFLECTION |
                                                  openpbr::FLOW_CONTINUOUS_TRANSMISSION)) != 0;
    var sun : VisibleLight = {};
    var lamp : VisibleLight = {};
    if (continuous &&
        !openpbr::is_discrete_only_flow<flow>(material.pbr, call.medium, material.transmitted)) {
        if ((light_mask & 1u) != 0) {
            let proposal = scene.sample_sun(hit.surface, sampler.sun(depth));
            sun = visible_light<flow, true>(scene, hit, material, view, call.medium, proposal,
                                            !observe);
            if (observe && sun.pdf > 0)
                signals.sun_ray = proposal.visibility;
        }
        if ((light_mask & 2u) != 0)
            lamp =
                visible_light<flow, false>(scene, hit, material, view, call.medium,
                                           scene.sample_light(hit.surface, sampler.light(depth)));
    }
    let scatter = continuation && !limited;
    var consumer : DirectEvaluationConsumer<observe> = { float4(sun.incident_radiance, sun.pdf),
                                                         float4(lamp.incident_radiance, lamp.pdf),
                                                         call.throughput,
                                                         call.radiance,
                                                         continuation,
                                                         0,
                                                         0 };
    var next : openpbr::Sample;
#if defined(OPENPBR_FLOW) || defined(GAME_FLOW)
    if (primary)
        next = openpbr::shade_primary_flow_consume<flow, openpbr::Energy,
                                                   DirectEvaluationConsumer<observe>,
                                                   continuous ? light_mask : 0u, observe>(
            scene.energy(), material.pbr, shading_normal(hit, material), view, sun.direction,
            lamp.direction, uint(sun.pdf > 0) | (uint(lamp.pdf > 0) << 1u),
            scatter ? sampler.bsdf(depth) : float3(0), scatter, call.medium, material.transmitted,
            2u, primary_lane, consumer);
    else
#endif
        next = openpbr::shade_flow_consume<flow, openpbr::Energy, DirectEvaluationConsumer<observe>,
                                           continuous ? light_mask : 0u>(
            scene.energy(), material.pbr, shading_normal(hit, material), view, sun.direction,
            lamp.direction, uint(sun.pdf > 0) | (uint(lamp.pdf > 0) << 1u),
            scatter ? sampler.bsdf(depth) : float3(0), scatter, call.medium, material.transmitted,
            consumer);
    let continues =
        scatter && next.events != 0 && scatter_side_supported(hit, material, view, next);
    call.radiance = consumer.radiance;
    if (observe) {
        signals.sun = consumer.direct_sun;
        signals.local = consumer.direct_local;
        signals.local_light = float4(lamp.direction, lamp.distance);
        signals.events = continues ? next.events : 0;
    }
    if (!continues) {
        call.direction = incoming;
        call.finish_vertex(depth, events,
                           limited && continuation ? PATH_DIELECTRIC_LIMIT : PATH_DONE);
        return;
    }
    if (solid && (next.events & openpbr::DISCRETE) != 0 &&
        dot(view, hit.surface.normal) * dot(next.direction, hit.surface.normal) < 0)
        next.response *= material.boundary_filter;
    call.throughput = transport::product_over(call.throughput, next.response, next.pdf);
    if (solid) {
        call.eta_scale = transport::eta_scale_after_scatter(
            call.eta_scale, next.relative_eta, (next.events & openpbr::TRANSMISSION) != 0);
        call.medium = next.medium;
    }
    call.direction = next.direction;
    call.previous_pdf = (next.events & openpbr::DISCRETE) != 0 ? 0 : next.pdf;
    call.previous = hit.surface.position;
    call.origin_pixel = geometry::offset_origin(hit.surface, next.direction);
    var status : uint = PATH_ACTIVE;
    if (max(call.throughput.x, max(call.throughput.y, call.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (depth + 1u >= 2u) {
        let survival = transport::roulette_probability(call.throughput, call.eta_scale);
        if (survival == 0 || (survival < 1 && sampler.roulette(depth) >= survival))
            status = PATH_DONE;
        else if (survival < 1)
            call.throughput = transport::roulette_reweight(call.throughput, survival);
    }
    call.finish_vertex(depth, events, status);
}

public func shade_material<let flow : uint, Scene : IFlowScene, let light_mask : uint = 3,
                           let primary : bool = false>(scene: Scene, inout call: MaterialCall,
                                                       hit: Hit,
                                                       material: transport::SurfaceMaterial,
                                                       incoming: float3, settings: Settings,
                                                       primary_lane: uint = 0)
    ->void {
    var unused : PrimaryLighting;
    shade_material_observed<flow, Scene, light_mask, primary, false>(
        scene, call, hit, material, incoming, settings, primary_lane, unused);
}
}
