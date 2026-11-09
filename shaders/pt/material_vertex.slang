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
    public float3 origin_pixel;
    public float3 direction;
    public float3 throughput;
    public float3 radiance;
    public float4 medium;
    public float3 previous;
    public float previous_pdf;
    public float eta_scale;
    public uint state;

    public static MaterialCall input(TransportCall data)
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
    public static MaterialCall primaryInput(uint sample, uint lane, float3 radiance)
    {
        MaterialCall call;
        call.origin_pixel = float3(asfloat(sample), asfloat(lane), 0);
        call.radiance = radiance;
        return call;
    }
    /// 首面叶子建立相机路径的不变量，避免调度器为常量初态搬运完整路径。
    [mutating]
    public void initializePrimary(float4 incident)
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
    public void finishVertex(uint depth, uint events, uint status)
    {
        if (!all(isfinite(throughput)) || !all(isfinite(radiance)))
            status = PATH_INVALID;
        state = status | ((depth + 1u) << 2u) | (events << 9u);
        bool active = status == PATH_ACTIVE;
        origin_pixel = select(active, origin_pixel, float3(0));
        previous = select(active, previous, float3(0));
        previous_pdf = select(active, previous_pdf, 0.0);
    }
    public PathState result()
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
    float3 direction;
    float3 incidentRadiance;
    float pdf;
};

VisibleLight visible_light<let Flow : uint, let Solar : bool, Scene : IFlowScene>(
    Scene scene, Hit hit, SurfaceMaterial material, float3 view, float4 incident, LightSample light)
{
    VisibleLight visible = {};
    if (light.pdf <= 0 ||
        !openpbr::may_evaluate_flow<Flow>(material.pbr, shading_normal(hit, material), view,
                                          light.direction, incident, material.transmitted) ||
        scene.occluded(light.visibility))
        return visible;
    float3 extinction = incident.yzw;
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
public void shade_material<let Flow : uint, Scene : IFlowScene, let LightMask : uint = 3,
                           let Primary : bool = false>(Scene scene, inout MaterialCall call,
                                                       Hit hit, SurfaceMaterial material,
                                                       float3 incoming, Settings settings,
                                                       uint primarySamples = 1,
                                                       uint primaryLane = 0)
{
    uint depth = call.state & 127u;
    uint events = (call.state >> 7u) & 127u;
    uint sample = asuint(call.origin_pixel.x);
    call.throughput *= segment_transmittance(call.medium.yzw, hit.distance);
    if ((Flow & openpbr::FLOW_EMISSION) != 0 && any(material.emission > 0))
    {
        float w = call.previous_pdf > 0
                      ? power_heuristic(call.previous_pdf, scene.emission_pdf(hit, call.previous))
                      : 1;
        call.radiance += call.throughput * material.emission * w;
    }
    bool continuation = depth < settings.budget;
    bool solid = (Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0;
    if (solid && continuation)
        events++;
    bool limited = solid && events > settings.dielectric_limit;
    float3 view = -incoming;
    PathSampler sampler =
        path_sampler(DispatchRaysIndex().xy, sample, settings.resolution, settings.seed);
    static const bool Continuous =
        (Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                 openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) != 0;
    VisibleLight sun = {}, lamp = {};
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
    bool scatter = continuation && !limited;
    openpbr::Batch bsdf;
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
    openpbr::Sample next = bsdf.scattered;
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
    uint status = PATH_ACTIVE;
    if (max(call.throughput.x, max(call.throughput.y, call.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (depth + 1u >= 2u)
    {
        float survival =
            roulette_probability(call.throughput, call.eta_scale, settings.square_root_roulette);
        if (survival == 0 || (survival < 1 && sampler.roulette(depth) >= survival))
            status = PATH_DONE;
        else if (survival < 1)
            call.throughput = roulette_reweight(call.throughput, survival);
    }
    call.finishVertex(depth, events, status);
}
}
