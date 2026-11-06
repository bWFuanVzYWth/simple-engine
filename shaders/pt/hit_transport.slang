/// 普通PT：HitObject/SER材质分流，100B交接，raygen中的独立RayQuery阴影。
module hit_transport;
import pt.material_vertex;
import pt.scene;
import pt.integrator;
import pt.transport;
import pt.roulette;
import pt.path_sampler;
import geometry.ray;
import bsdf.openpbr.api;

#ifndef PT_HIT_SER
#define PT_HIT_SER 0
#endif

namespace pt
{
/// 应用仅适配实际命中、材质身份与能力；材质读取/解码仍属于各自场景。
/// context.state低30bit必须无损容纳应用身份；program对应已准备的材质Hit Group。
public interface IHitScene : IFlowScene
{
    RaytracingAccelerationStructure acceleration();
    Hit load_hit(HitObject object, float3 direction);
    void resolve_hit(inout Hit hit, float4 incident);
    uint program(Hit hit);
    uint reorder_bits();
    MaterialContext material_context(Hit hit);
    bool continuous(Hit hit);
    bool may_evaluate_both_sides(Hit hit);
    bool solid(Hit hit);
};

/// 已知初始介质与随机身份；只发布完整路径结果，guides观察独立于照明终止。
public float trace_hit_path<Scene : IHitScene, Guides : IPrefixGuides>(
    Scene provider, Guides guides, inout TransportCall data, PathSampler sampler, Settings settings)
{
    float distance = FAR;
    while (data.path.status == PATH_ACTIVE)
    {
        RayDesc ray = { data.path.origin, 0, data.path.direction, FAR };
        uint unused = 0;
        // HitObject必须为独立局部变量，不放入聚合状态（Slang 2026.20驱动互操作约束）。
        HitObject found =
            HitObject::TraceRay(provider.acceleration(), RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, unused);
        if (!found.IsHit())
        {
            if (data.path.depth == 0)
                guides.environment(data);
            escape(provider, data.path);
            break;
        }
        data.hit = provider.load_hit(found, data.path.direction);
        provider.resolve_hit(data.hit, data.path.medium);
        if (data.path.depth == 0)
        {
            distance = data.hit.distance;
            guides.surface(data);
        }
        data.path.throughput *= segment_transmittance(data.path.medium.yzw, data.hit.distance);
        LightSample sun = {}, lamp = {};
        float3 solar = 0, local = 0;
        if (provider.continuous(data.hit))
        {
            sun = provider.sample_sun(data.hit.surface, sampler.sun(data.path.depth));
            lamp = provider.sample_light(data.hit.surface, sampler.light(data.path.depth));
            bool both_sides = provider.may_evaluate_both_sides(data.hit);
            if (sun.pdf > 0 &&
                (both_sides || dot(-data.path.direction, data.hit.surface.normal) *
                                       dot(sun.direction, data.hit.surface.normal) >
                                   0) &&
                !provider.occluded(sun.visibility))
                solar = provider.light_emission(data.hit.surface, sun, true);
            if (lamp.pdf > 0 &&
                (both_sides || dot(-data.path.direction, data.hit.surface.normal) *
                                       dot(lamp.direction, data.hit.surface.normal) >
                                   0) &&
                !provider.occluded(lamp.visibility))
                local = provider.light_emission(data.hit.surface, lamp, false);
            // 零贡献方向不进入材质求值，避免遮挡光仍产生无效BSDF计算。
            if (!any(solar > 0))
                sun.pdf = 0;
            if (!any(local > 0))
                lamp.pdf = 0;
        }
        bool continuation = data.path.depth < settings.budget;
        bool solid = provider.solid(data.hit);
        bool limited = solid && data.path.dielectric_events + 1 > settings.dielectric_limit;
        MaterialCall call = MaterialCall::input(provider.material_context(data.hit),
                                                data.path.medium, sampler.bsdf(data.path.depth),
                                                sun, lamp, continuation && !limited, continuation);
        uint program = provider.program(data.hit);
        found.SetShaderTableIndex(program + 1u);
#if PT_HIT_SER
        ReorderThread(program, provider.reorder_bits());
#endif
        HitObject::Invoke(found, call);
        MaterialVertex result = call.result();
        float emission_weight =
            any(result.emission > 0) && data.path.previous_pdf > 0
                ? power_heuristic(data.path.previous_pdf,
                                  provider.emission_pdf(data.hit, data.path.previous))
                : 1;
        data.path.radiance += data.path.throughput * (result.emission * emission_weight +
                                                      result.sun * solar + result.lamp * local);
        if (!continuation)
            data.path.status = PATH_DONE;
        else if (limited)
        {
            data.path.dielectric_events += 1;
            data.path.status = PATH_DIELECTRIC_LIMIT;
        }
        else if (result.events == 0)
            data.path.status = PATH_DONE;
        else
        {
            data.path.direction = result.direction;
            data.path.throughput =
                product_over(data.path.throughput, result.response_pdf.xyz, result.response_pdf.w);
            if (solid)
            {
                data.path.dielectric_events += 1;
                data.path.eta_scale =
                    eta_scale_after_scatter(data.path.eta_scale, result.relative_eta,
                                            (result.events & openpbr::TRANSMISSION) != 0);
                data.path.medium = result.medium;
            }
            data.path.previous_pdf =
                (result.events & openpbr::DISCRETE) != 0 ? 0 : result.response_pdf.w;
            data.path.previous = data.hit.surface.position;
            data.path.origin = geometry::offset_origin(data.hit.surface, data.path.direction);
        }
        uint status = data.path.status;
        if (status == PATH_ACTIVE)
        {
            if (max(data.path.throughput.x, max(data.path.throughput.y, data.path.throughput.z)) <=
                0)
                status = PATH_DONE;
            else if (data.path.depth + 1 >= 2)
            {
                float survival = roulette_probability(data.path.throughput, data.path.eta_scale);
                if (survival == 0 ||
                    (survival < 1 && sampler.roulette(data.path.depth) >= survival))
                    status = PATH_DONE;
                else if (survival < 1)
                    data.path.throughput = roulette_reweight(data.path.throughput, survival);
            }
        }
        finish(data.path, status);
    }
    return distance;
}
}
