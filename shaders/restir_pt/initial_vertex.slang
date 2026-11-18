/// 初采的完整材质控制域；Flow裁掉无消费者的选灯、纹理、BSDF与阴影。
module initial_vertex;
import restir_pt.scene;
import restir_pt.types;
import restir_pt.initial_random;
import restir_pt.material;
import restir_pt.replay;
import openpbr.flow;
import transport.surface_material;

namespace restir_pt {
func consider_direct<Scene : IRestirScene>(inout vertex: InitialVertex, family: uint, scene: Scene,
                                           surface: Surface, light: LightSample,
                                           nee: BsdfEvaluation, transmittance: float3,
                                           request: InitialRequest)
    ->void {
    if (!(light.pdf > 0.0) || !any(light.radiance > 0.0))
        return;
    let mis = (request.flags & INITIAL_MIS) != 0 && light.light_type != LIGHT_ANALYTIC
                  ? light_mis_weight(1.0, light.pdf, 1.0, nee.pdf)
                  : 1.0;
    let reflected = nee.value * light.radiance * transmittance * (mis / light.pdf);
    if (!any(reflected > 0.0))
        return;
    var target : Surface = {};
    if (light.hit.is_valid()) {
        target.hit = light.hit;
        target.point = scene.light_endpoint(light.hit, false);
    }
    let visible = light.hit.is_valid() ? scene.visible(surface, target, false)
                                       : scene.visible_direction(surface, light.direction, false);
    if (!visible)
        return;
    if (!PathRISState::select_candidate(vertex.weight,
                                        initial_light_ris_random(request.bounce_seed, family),
                                        request.throughput * reflected))
        return;
    if ((request.flags & INITIAL_PRIMARY) == 0)
        vertex.emission = 0;
    let point = light.hit.is_valid() && light.light_type == LIGHT_ANALYTIC &&
                scene.is_point_light(light.hit);
    let factor = light.hit.is_valid()
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
                      light.light_type | (point ? INITIAL_POINT_LIGHT : 0u) };
}

// 初采只有一个路径内 reservoir。按原随机维逐项更新和，胜者以外的重连数据在叶子内释放。
func select_hit(inout result: InitialVertex, request: InitialRequest, emission: float3, pdf: float)
    ->void {
    if ((request.flags & INITIAL_PRIMARY) != 0)
        result.emission = emission;
    else if (any(emission > 0.0)) {
        let mis = request.previous_mis_pdf >= 0.0
                      ? light_mis_weight(1.0, request.previous_mis_pdf, 1.0, pdf)
                      : 1.0;
        if (PathRISState::select_candidate(result.weight,
                                           initial_random_value<1>(request.bounce_seed),
                                           request.throughput * mis * emission)) {
            result.emission = emission;
            result.emission_pdf = pdf;
        }
    }
}

/// 专用叶子已解码一次完整材料；source的point始终为几何法线/误差界，material.normal仅供BSDF。
/// 两族只采一次，候选可见性在已知精确BSDF支持之后查询；不跨调用传射线或材料闭包。
public func shade_initial_vertex<let flow : uint, Scene : IRestirScene, let light_mask : uint = 3>(
    scene: Scene, surface: Surface, material: transport::SurfaceMaterial, energy: openpbr::Energy,
    request: InitialRequest, emission_pdf: float)
    ->InitialVertex {
    static const let directions : uint = (flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                                                  openpbr::FLOW_CONTINUOUS_REFLECTION |
                                                  openpbr::FLOW_CONTINUOUS_TRANSMISSION)) != 0
                                             ? light_mask
                                             : 0u;
    var result : InitialVertex = {};
    result.weight = request.weight;
    if ((flow & openpbr::FLOW_EMISSION) != 0)
        select_hit(result, request, material.emission, emission_pdf);
    var first : LightSample = {};
    var second : LightSample = {};
    var evaluation : uint = 0;
    if (directions != 0 && (request.flags & INITIAL_NEE) != 0 &&
        !openpbr::is_discrete_only_flow<flow>(material.pbr, surface.incident,
                                              material.transmitted)) {
        let two : bool = (request.flags & INITIAL_TWO_LIGHTS) != 0;
        if ((directions & 1u) != 0) {
            first =
                scene.sample_light(surface, two ? initial_light_random<2>(request.bounce_seed, 0)
                                                : initial_light_random<1>(request.bounce_seed, 0));
            if (first.pdf > 0)
                evaluation |= 1u;
        }
        if ((directions & 2u) != 0 && two) {
            second = scene.sample_light(surface, initial_light_random<2>(request.bounce_seed, 1));
            if (second.pdf > 0)
                evaluation |= 2u;
        }
    }
    let scatter : bool = (request.flags & INITIAL_SAMPLE) != 0;
    let random : float3 = scatter ? initial_bsdf_random(request.bounce_seed,
                                                        (request.flags & INITIAL_RR) != 0)
                                  : float3(0);
    let normal = material_normal(surface, material);
    let batch : openpbr::Batch = openpbr::shade_flow<flow, openpbr::Energy, directions>(
                    energy, material.pbr, normal, surface.wo, first.direction, second.direction,
                    evaluation, random, scatter, surface.incident, material.transmitted);
    result.sample = material_sample<flow>(surface, material, batch.scattered);
    if (directions != 0) {
        if ((directions & 1u) != 0 && first.pdf > 0) {
            consider_direct(
                result, 0u, scene, surface, first, { batch.first.response, batch.first.pdf },
                material_transmittance<flow>(surface, material, first.direction, first.distance),
                request);
        }
        if ((directions & 2u) != 0 && second.pdf > 0) {
            consider_direct(
                result, 1u, scene, surface, second, { batch.second.response, batch.second.pdf },
                material_transmittance<flow>(surface, material, second.direction, second.distance),
                request);
        }
    }
    return result;
}

/// 解析场景/测试的非分派适配；缓存两族样本，只调用一次现有材料求值，不重复选灯。
public func generic_initial_vertex<Scene : IRestirScene>(scene: Scene, surface: Surface,
                                                         request: InitialRequest)
    ->InitialVertex {
    let sample_nee : bool = (request.flags & INITIAL_NEE) != 0 && surface.has_non_delta_lobes;
    let two : bool = (request.flags & INITIAL_TWO_LIGHTS) != 0;
    var first : LightSample = {};
    var second : LightSample = {};
    if (sample_nee) {
        first = scene.sample_light(surface, two ? initial_light_random<2>(request.bounce_seed, 0)
                                                : initial_light_random<1>(request.bounce_seed, 0));
        if (two)
            second = scene.sample_light(surface, initial_light_random<2>(request.bounce_seed, 1));
    }
    let scatter : bool = (request.flags & INITIAL_SAMPLE) != 0;
    let shading
        : InitialShading = scene.shade_initial(
              surface,
              scatter ? initial_bsdf_random(request.bounce_seed, (request.flags & INITIAL_RR) != 0)
                      : float3(0),
              first.pdf > 0 ? first.direction : surface.wo,
              second.pdf > 0 ? second.direction : surface.wo, scatter);
    var result : InitialVertex = {};
    result.weight = request.weight;
    result.sample = shading.sample;
    var pdf : float = 0;
    if ((request.flags & INITIAL_PRIMARY) == 0 && any(shading.emission > 0)) {
        var previous : Surface = {};
        previous.point.position = request.previous_position;
        pdf = scene.light_pdf(previous, surface.hit, -surface.wo, LIGHT_EMISSIVE, false);
    }
    select_hit(result, request, shading.emission, pdf);
    if (first.pdf > 0)
        consider_direct(result, 0u, scene, surface, first, shading.nee,
                        scene.transmittance(surface, first.direction, first.distance), request);
    if (second.pdf > 0)
        consider_direct(result, 1u, scene, surface, second, shading.second_nee,
                        scene.transmittance(surface, second.direction, second.distance), request);
    return result;
}
}
