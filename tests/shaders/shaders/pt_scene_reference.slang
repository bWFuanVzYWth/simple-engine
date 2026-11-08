/// 冻结的旧积分逻辑，仅用于独立测试对照，不被生产入口导入。
module pt_scene_reference;
import pt.integrator;
import pt.transport;
import bsdf.openpbr.specialized;
import bsdf.openpbr.api;
import pt.roulette;
import geometry.ray;
namespace pt
{
float3 connection_medium<let Capability : uint>(Hit hit, float3 view, float3 direction,
                                                float4 incident, float4 transmitted)
{
    if (Capability == openpbr::CAP_SOLID_DIELECTRIC)
    {
        bool crossing = dot(view, hit.surface.normal) * dot(direction, hit.surface.normal) < 0;
        return crossing ? transmitted.yzw : incident.yzw;
    }
    return incident.yzw;
}

void direct_sample<let Capability : uint, let Continuous : bool, Scene>(
    Scene scene, Hit hit, SurfaceMaterial material, float3 view, LightSample light, bool solar,
    bool continuation, inout PathState path)
    where Scene : IScene
{
    if (light.pdf <= 0)
        return;
    if (!openpbr::may_evaluate_specialized<Capability>(material.pbr, shading_normal(hit, material),
                                                       view, light.direction, path.medium,
                                                       material.transmitted))
        return;
    if (scene.occluded(light.visibility))
        return;
    float3 t =
        segment_transmittance(connection_medium<Capability>(hit, view, light.direction, path.medium,
                                                            material.transmitted),
                              light.distance);
    openpbr::Evaluation evaluated;
    if (!Continuous)
        evaluated = openpbr::evaluate_specialized<Capability>(
            scene.energy(), material.pbr, shading_normal(hit, material), view, light.direction,
            path.medium, material.transmitted);
    else if (Capability == openpbr::CAP_SOLID_DIELECTRIC)
        evaluated = openpbr::evaluate_solid_continuous(
            scene.energy(), material.pbr, shading_normal(hit, material), view, light.direction,
            path.medium, material.transmitted);
    else
        evaluated = openpbr::evaluate_opaque_continuous<Capability>(
            scene.energy(), material.pbr, shading_normal(hit, material), view, light.direction,
            path.medium, material.transmitted);
    if (any(evaluated.response > 0))
    {
        float weight = power_heuristic_over_pdf(light.pdf, continuation ? evaluated.pdf : 0);
        float3 emission = scene.light_emission(hit.surface, light, solar);
        path.radiance += path.throughput * triple_product(evaluated.response, emission * t, weight);
    }
}

void reference_accumulate_emission<Scene>(Scene scene, Hit hit, float3 emission,
                                          inout PathState path)
    where Scene : IScene
{
    if (any(emission > 0))
    {
        float weight =
            path.previous_pdf > 0
                ? power_heuristic(path.previous_pdf, scene.emission_pdf(hit, path.previous))
                : 1;
        path.radiance += path.throughput * emission * weight;
    }
}

/// 消费当前段和顶点，不查询下一表面；同一函数供 delta 前缀和叶子 callable 使用。
/// Continuous=true 只适用于已证明的 opaque roughness² 下界／非匹配 solid IOR 窄域。
/// 运行时任意合法材料应使用默认 false，避免把应用固定参数推广为通用前提。
public void process_reference_material<let Capability : uint, let Emissive : bool = false,
                                       let Continuous : bool = false, Scene>(
    Scene scene, inout TransportCall data, Settings settings)
    where Scene : IScene
{
    PathState path = {};
    path.direction = data.path.direction;
    path.throughput = data.path.throughput;
    path.radiance = data.path.radiance;
    path.medium = data.path.medium;
    path.eta_scale = data.path.eta_scale;
    path.depth = data.path.depth;
    path.dielectric_events = data.path.dielectric_events;
    if (Emissive)
    {
        path.previous = data.path.previous;
        path.previous_pdf = data.path.previous_pdf;
    }
    Hit hit = data.hit;
    path.throughput *= segment_transmittance(path.medium.yzw, hit.distance);
    SurfaceMaterial material = scene.resolve(hit, Capability, path.medium);
    if (Emissive)
        reference_accumulate_emission(scene, hit, material.emission, path);
    float3 view = -path.direction;
    bool continuation = path.depth < settings.budget;
    PathSampler sampler = path_sampler(data.pixel, data.sample, settings.resolution, settings.seed);
    if (Capability < openpbr::CAP_DELTA_CONDUCTOR)
    {
        direct_sample<Capability, Continuous>(
            scene, hit, material, view, scene.sample_sun(hit.surface, sampler.sun(path.depth)),
            true, continuation, path);
        direct_sample<Capability, Continuous>(
            scene, hit, material, view, scene.sample_light(hit.surface, sampler.light(path.depth)),
            false, continuation, path);
    }
    if (!continuation)
    {
        finish(path, PATH_DONE);
        data.path = path;
        return;
    }
    if (Capability == openpbr::CAP_SOLID_DIELECTRIC ||
        Capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC)
    {
        path.dielectric_events += 1;
        if (path.dielectric_events > settings.dielectric_limit)
        {
            finish(path, PATH_DIELECTRIC_LIMIT);
            data.path = path;
            return;
        }
    }
    openpbr::Sample scattered;
    if (Capability >= openpbr::CAP_DELTA_CONDUCTOR)
        scattered = openpbr::sample_delta_specialized<Capability>(
            material.pbr, shading_normal(hit, material), view, sampler.bsdf(path.depth).z,
            path.medium, material.transmitted);
    else if (!Continuous)
        scattered = openpbr::sample_specialized<Capability>(
            scene.energy(), material.pbr, shading_normal(hit, material), view,
            sampler.bsdf(path.depth), path.medium, material.transmitted);
    else if (Capability == openpbr::CAP_SOLID_DIELECTRIC)
        scattered = openpbr::sample_solid_continuous(
            scene.energy(), material.pbr, shading_normal(hit, material), view,
            sampler.bsdf(path.depth), path.medium, material.transmitted);
    else
        scattered = openpbr::sample_opaque_continuous<Capability>(
            scene.energy(), material.pbr, shading_normal(hit, material), view,
            sampler.bsdf(path.depth), path.medium, material.transmitted);
    if (scattered.events == 0)
    {
        finish(path, PATH_DONE);
        data.path = path;
        return;
    }
    float3 response = scattered.response;
    if (Capability == openpbr::CAP_DELTA_SOLID_DIELECTRIC &&
        dot(view, hit.surface.normal) * dot(scattered.direction, hit.surface.normal) < 0)
        response *= material.boundary_filter;
    path.direction = scattered.direction;
    path.throughput = product_over(path.throughput, response, scattered.pdf);
    bool transmission = (scattered.events & openpbr::TRANSMISSION) != 0;
    path.eta_scale = eta_scale_after_scatter(path.eta_scale, scattered.relative_eta, transmission);
    path.medium = scattered.medium;
    if (Continuous)
        path.previous_pdf = Capability >= openpbr::CAP_DELTA_CONDUCTOR ? 0 : scattered.pdf;
    else
        path.previous_pdf = (scattered.events & openpbr::DISCRETE) != 0 ? 0 : scattered.pdf;
    path.previous = hit.surface.position;
    path.origin = geometry::offset_origin(hit.surface, path.direction);
    uint status = PATH_ACTIVE;
    if (max(path.throughput.x, max(path.throughput.y, path.throughput.z)) <= 0)
        status = PATH_DONE;
    else if (path.depth + 1 >= 2)
    {
        float survival = roulette_probability(path.throughput, path.eta_scale);
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

}
