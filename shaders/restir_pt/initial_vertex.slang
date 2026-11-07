/// 初采的完整材质控制域；Flow裁掉无消费者的选灯、纹理、BSDF与阴影。
module initial_vertex;
import restir_pt.scene;
import restir_pt.types;
import restir_pt.initial_random;
import restir_pt.material;
import restir_pt.replay;
import pt.scene;
import pt.transport;
import bsdf.openpbr.flow;

namespace restir_pt
{
void consider_direct<Scene : IRestirScene>(inout InitialVertex vertex, uint family, Scene scene,
                                           Surface surface, LightSample light, BsdfEvaluation nee,
                                           float3 transmittance, InitialRequest request)
{
    if (!(light.pdf > 0.0) || !any(light.radiance > 0.0))
        return;
    float mis = (request.flags & INITIAL_MIS) != 0 && light.lightType != LIGHT_ANALYTIC
                    ? lightMisWeight(1.0, light.pdf, 1.0, nee.pdf)
                    : 1.0;
    float3 reflected = nee.value * light.radiance * transmittance * (mis / light.pdf);
    if (!any(reflected > 0.0))
        return;
    Surface target = {};
    if (light.hit.isValid())
    {
        target.hit = light.hit;
        target.point = scene.lightEndpoint(light.hit, false);
    }
    bool visible = light.hit.isValid() ? scene.visible(surface, target, false)
                                       : scene.visibleDirection(surface, light.direction, false);
    if (!visible)
        return;
    if (!PathRISState::selectCandidate(vertex.weight,
                                       initialLightRISRandom(request.bounceSeed, family),
                                       request.throughput * reflected))
        return;
    if ((request.flags & INITIAL_PRIMARY) == 0)
        vertex.emission = 0;
    bool point =
        light.hit.isValid() && light.lightType == LIGHT_ANALYTIC && scene.isPointLight(light.hit);
    float factor = light.hit.isValid()
                       ? (point ? 1.0 : abs(dot(target.point.normal, -light.direction))) /
                             (light.distance * light.distance)
                       : 1.0;
    vertex.direct = { light.hit,
                      light.direction,
                      light.radiance,
                      reflected,
                      light.radiance * transmittance,
                      light.pdf,
                      nee.pdf,
                      factor,
                      light.lightType | (point ? INITIAL_POINT_LIGHT : 0u) };
}

// 初采只有一个路径内 reservoir。按原随机维逐项更新和，胜者以外的重连数据在叶子内释放。
void select_hit(inout InitialVertex result, InitialRequest request, float3 emission, float pdf)
{
    if ((request.flags & INITIAL_PRIMARY) != 0)
        result.emission = emission;
    else if (any(emission > 0.0))
    {
        float mis = request.previousMisPdf >= 0.0
                        ? lightMisWeight(1.0, request.previousMisPdf, 1.0, pdf)
                        : 1.0;
        if (PathRISState::selectCandidate(result.weight, initialRandomValue<1>(request.bounceSeed),
                                          request.throughput * mis * emission))
        {
            result.emission = emission;
            result.emissionPdf = pdf;
        }
    }
}

/// 专用叶子已解码一次完整材料；source的point始终为几何法线/误差界，material.normal仅供BSDF。
/// 两族只采一次，候选可见性在已知精确BSDF支持之后查询；不跨调用传射线或材料闭包。
public InitialVertex shade_initial_vertex<let Flow : uint, Scene : IRestirScene,
                                          let LightMask : uint = 3>(Scene scene, Surface surface,
                                                                    pt::SurfaceMaterial material,
                                                                    openpbr::Energy energy,
                                                                    InitialRequest request,
                                                                    float emissionPdf)
{
    static const uint Directions =
        (Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                 openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) != 0
            ? LightMask
            : 0u;
    InitialVertex result = {};
    result.weight = request.weight;
    if ((Flow & openpbr::FLOW_EMISSION) != 0)
        select_hit(result, request, material.emission, emissionPdf);
    LightSample first = {}, second = {};
    uint evaluation = 0;
    if (Directions != 0 && (request.flags & INITIAL_NEE) != 0 &&
        !openpbr::is_discrete_only_flow<Flow>(material.pbr, surface.incident, material.transmitted))
    {
        bool two = (request.flags & INITIAL_TWO_LIGHTS) != 0;
        if ((Directions & 1u) != 0)
        {
            first = scene.sampleLight(surface, two ? initialLightRandom<2>(request.bounceSeed, 0)
                                                   : initialLightRandom<1>(request.bounceSeed, 0));
            if (first.pdf > 0)
                evaluation |= 1u;
        }
        if ((Directions & 2u) != 0 && two)
        {
            second = scene.sampleLight(surface, initialLightRandom<2>(request.bounceSeed, 1));
            if (second.pdf > 0)
                evaluation |= 2u;
        }
    }
    bool scatter = (request.flags & INITIAL_SAMPLE) != 0;
    float3 random = scatter
                        ? initialBsdfRandom(request.bounceSeed, (request.flags & INITIAL_RR) != 0)
                        : float3(0);
    float3 normal = material_normal(surface, material);
    openpbr::Batch batch = openpbr::shade_flow<Flow, openpbr::Energy, Directions>(
        energy, material.pbr, normal, surface.wo, first.direction, second.direction, evaluation,
        random, scatter, surface.incident, material.transmitted);
    result.sample = material_sample<Flow>(surface, material, batch.scattered);
    if (Directions != 0)
    {
        if ((Directions & 1u) != 0 && first.pdf > 0)
        {
            consider_direct(
                result, 0u, scene, surface, first, { batch.first.response, batch.first.pdf },
                material_transmittance<Flow>(surface, material, first.direction, first.distance),
                request);
        }
        if ((Directions & 2u) != 0 && second.pdf > 0)
        {
            consider_direct(
                result, 1u, scene, surface, second, { batch.second.response, batch.second.pdf },
                material_transmittance<Flow>(surface, material, second.direction, second.distance),
                request);
        }
    }
    return result;
}

/// 解析场景/测试的非分派适配；缓存两族样本，只调用一次现有材料求值，不重复选灯。
public InitialVertex generic_initial_vertex<Scene : IRestirScene>(Scene scene, Surface surface,
                                                                  InitialRequest request)
{
    bool sampleNEE = (request.flags & INITIAL_NEE) != 0 && surface.hasNonDeltaLobes;
    bool two = (request.flags & INITIAL_TWO_LIGHTS) != 0;
    LightSample first = {}, second = {};
    if (sampleNEE)
    {
        first = scene.sampleLight(surface, two ? initialLightRandom<2>(request.bounceSeed, 0)
                                               : initialLightRandom<1>(request.bounceSeed, 0));
        if (two)
            second = scene.sampleLight(surface, initialLightRandom<2>(request.bounceSeed, 1));
    }
    bool scatter = (request.flags & INITIAL_SAMPLE) != 0;
    InitialShading shading = scene.shadeInitial(
        surface,
        scatter ? initialBsdfRandom(request.bounceSeed, (request.flags & INITIAL_RR) != 0)
                : float3(0),
        first.pdf > 0 ? first.direction : surface.wo,
        second.pdf > 0 ? second.direction : surface.wo, scatter);
    InitialVertex result = {};
    result.weight = request.weight;
    result.sample = shading.sample;
    float pdf = 0;
    if ((request.flags & INITIAL_PRIMARY) == 0 && any(shading.emission > 0))
    {
        Surface previous = {};
        previous.point.position = request.previousPosition;
        pdf = scene.lightPdf(previous, surface.hit, -surface.wo, LIGHT_EMISSIVE, false);
    }
    select_hit(result, request, shading.emission, pdf);
    if (first.pdf > 0)
        consider_direct(result, 0u, scene, surface, first, shading.nee,
                        scene.transmittance(surface, first.direction, first.distance), request);
    if (second.pdf > 0)
        consider_direct(result, 1u, scene, surface, second, shading.secondNEE,
                        scene.transmittance(surface, second.direction, second.distance), request);
    return result;
}
}
