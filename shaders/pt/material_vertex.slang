/// 完整材质顶点的closest-hit边界；几何和材质只在已经特化的命中程序中解码。
module material_vertex;
import pt.scene;
import pt.integrator;
import pt.path_sampler;
import pt.transport;
import pt.roulette;
import bsdf.openpbr.flow;
import geometry.ray;

namespace pt
{
/// 88B无损逻辑交接；输入样本号复用输出origin.x，像素直接取发射内建量，不搬运Hit、纹素、法线或灯样本。
/// 输入state为depth[0..6]/dielectric_events[7..13]；输出为status[0..1]/depth/events。
/// 同一次 Invoke 内先消费入射状态，再发布出射状态。
public struct MaterialCall
{
    public var origin_pixel : float3;
    public var direction : float3;
    public var throughput : float3;
    public var radiance : float3;
    public var medium : float4;
    public var previous : float3;
    public var previous_pdf : float;
    public var eta_scale : float;
    public var state : uint;

    public static func input(data: TransportCall)->MaterialCall
    {
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
    public static func primaryInput(sample_value: uint, lane: uint, radiance: float3)->MaterialCall
    {
        var call : MaterialCall;
        call.origin_pixel = float3(asfloat(sample_value), asfloat(lane), 0);
        call.radiance = radiance;
        return call;
    }
    /// 首面叶子建立相机路径的不变量，避免调度器为常量初态搬运完整路径。
    [mutating]
    public func initializePrimary(incident: float4)->void
    {
        direction = 0;
        throughput = 1;
        medium = incident;
        previous = 0;
        previous_pdf = 0;
        eta_scale = 1;
        state = 0;
    }
    [mutating]
    public func finishVertex(depth: uint, events: uint, status: uint)->void
    {
        var status = status;
        if (!all(isfinite(throughput)) || !all(isfinite(radiance)))
            status = PATH_INVALID;
        state = status | ((depth + 1u) << 2u) | (events << 9u);
        let active : bool = status == PATH_ACTIVE;
        origin_pixel = select(active, origin_pixel, float3(0));
        previous = select(active, previous, float3(0));
        previous_pdf = select(active, previous_pdf, 0.0);
    }
    public func result()->PathState
    {
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
struct VisibleLight
{
    var direction : float3;
    var incidentRadiance : float3;
    var pdf : float;
};

func visible_light<let Flow : uint, let Solar : bool, Scene : IFlowScene>(scene: Scene, hit: Hit,
                                                                          material: SurfaceMaterial,
                                                                          view: float3,
                                                                          incident: float4,
                                                                          light: LightSample)
    ->VisibleLight
{
    var visible : VisibleLight = {};
    if (light.pdf <= 0 ||
        !openpbr::may_evaluate_flow<Flow>(material.pbr, shading_normal(hit, material), view,
                                          light.direction, incident, material.transmitted) ||
        scene.occluded(light.visibility))
        return visible;
    var extinction : float3 = incident.yzw;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0 &&
        dot(view, hit.surface.normal) * dot(light.direction, hit.surface.normal) < 0)
        extinction = material.transmitted.yzw;
    visible.incidentRadiance = scene.light_emission(hit.surface, light, Solar) *
                               segment_transmittance(extinction, light.distance);
    visible.direction = light.direction;
    visible.pdf = light.pdf;
    return visible;
}

/// 已解码的本次命中由一个材质入口消费；路径结果直接原位发布。
public func shade_material<let Flow : uint, Scene : IFlowScene, let LightMask : uint = 3,
                           let Primary : bool = false>(scene: Scene, inout call: MaterialCall,
                                                       hit: Hit, material: SurfaceMaterial,
                                                       incoming: float3, settings: Settings,
                                                       primarySamples: uint = 1,
                                                       primaryLane: uint = 0)
    ->void
{
    let depth : uint = call.state & 127u;
    var events : uint = (call.state >> 7u) & 127u;
    let sample : uint = asuint(call.origin_pixel.x);
    call.throughput *= segment_transmittance(call.medium.yzw, hit.distance);
    if ((Flow & openpbr::FLOW_EMISSION) != 0 && any(material.emission > 0))
    {
        let w : float =
                    call.previous_pdf > 0
                        ? power_heuristic(call.previous_pdf, scene.emission_pdf(hit, call.previous))
                        : 1;
        call.radiance += call.throughput * material.emission * w;
    }
    let continuation : bool = depth < settings.budget;
    let solid : bool = (Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0;
    if (solid && continuation)
        events++;
    let limited : bool = solid && events > settings.dielectric_limit;
    let view : float3 = -incoming;
    let sampler : PathSampler = path_sampler(DispatchRaysIndex().xy, sample, settings.resolution,
                                             settings.seed);
    static const let Continuous : bool = (Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                                                  openpbr::FLOW_CONTINUOUS_REFLECTION |
                                                  openpbr::FLOW_CONTINUOUS_TRANSMISSION)) != 0;
    var sun : VisibleLight = {};
    var lamp : VisibleLight = {};
    if (Continuous &&
        !openpbr::is_discrete_only_flow<Flow>(material.pbr, call.medium, material.transmitted))
    {
        if ((LightMask & 1u) != 0)
            sun = visible_light<Flow, true>(scene, hit, material, view, call.medium,
                                            scene.sample_sun(hit.surface, sampler.sun(depth)));
        if ((LightMask & 2u) != 0)
            lamp =
                visible_light<Flow, false>(scene, hit, material, view, call.medium,
                                           scene.sample_light(hit.surface, sampler.light(depth)));
    }
    let scatter : bool = continuation && !limited;
    var bsdf : openpbr::Batch;
#if defined(OPENPBR_FLOW) || defined(GAME_FLOW)
    if (Primary)
        bsdf = openpbr::shade_primary_flow<Flow, openpbr::Energy, Continuous ? LightMask : 0u>(
            scene.energy(), material.pbr, shading_normal(hit, material), view, sun.direction,
            lamp.direction, uint(sun.pdf > 0) | (uint(lamp.pdf > 0) << 1u),
            scatter ? sampler.bsdf(depth) : float3(0), scatter, call.medium, material.transmitted,
            primarySamples, primaryLane);
    else
#endif
        bsdf = openpbr::shade_flow<Flow, openpbr::Energy, Continuous ? LightMask : 0u>(
            scene.energy(), material.pbr, shading_normal(hit, material), view, sun.direction,
            lamp.direction, uint(sun.pdf > 0) | (uint(lamp.pdf > 0) << 1u),
            scatter ? sampler.bsdf(depth) : float3(0), scatter, call.medium, material.transmitted);
    if (Continuous && (LightMask & 1u) != 0 && sun.pdf > 0 && any(bsdf.first.response > 0))
        call.radiance +=
            call.throughput *
            triple_product(bsdf.first.response, sun.incidentRadiance,
                           power_heuristic_over_pdf(sun.pdf, continuation ? bsdf.first.pdf : 0));
    if (Continuous && (LightMask & 2u) != 0 && lamp.pdf > 0 && any(bsdf.second.response > 0))
        call.radiance +=
            call.throughput *
            triple_product(bsdf.second.response, lamp.incidentRadiance,
                           power_heuristic_over_pdf(lamp.pdf, continuation ? bsdf.second.pdf : 0));
    var next : openpbr::Sample = bsdf.scattered;
    if (!scatter || next.events == 0)
    {
        call.direction = incoming;
        call.finishVertex(depth, events,
                          limited && continuation ? PATH_DIELECTRIC_LIMIT : PATH_DONE);
        return;
    }
    if (solid && (next.events & openpbr::DISCRETE) != 0 &&
        dot(view, hit.surface.normal) * dot(next.direction, hit.surface.normal) < 0)
        next.response *= material.boundary_filter;
    call.throughput = product_over(call.throughput, next.response, next.pdf);
    if (solid)
    {
        call.eta_scale = eta_scale_after_scatter(call.eta_scale, next.relative_eta,
                                                 (next.events & openpbr::TRANSMISSION) != 0);
        call.medium = next.medium;
    }
    call.direction = next.direction;
    call.previous_pdf = (next.events & openpbr::DISCRETE) != 0 ? 0 : next.pdf;
    call.previous = hit.surface.position;
    call.origin_pixel = geometry::offset_origin(hit.surface, next.direction);
    var status : uint = PATH_ACTIVE;
    if (max(call.throughput.x, max(call.throughput.y, call.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (depth + 1u >= 2u)
    {
        let survival : float = roulette_probability(call.throughput, call.eta_scale,
                                                    settings.square_root_roulette);
        if (survival == 0 || (survival < 1 && sampler.roulette(depth) >= survival))
            status = PATH_DONE;
        else if (survival < 1)
            call.throughput = roulette_reweight(call.throughput, survival);
    }
    call.finishVertex(depth, events, status);
}
}
