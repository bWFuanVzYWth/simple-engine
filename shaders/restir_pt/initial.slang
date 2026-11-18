/***************************************************************************
 # Copyright (c) 2015-26, NVIDIA CORPORATION. All rights reserved.
 #
 # Redistribution and use in source and binary forms, with or without
 # modification, are permitted provided that the following conditions
 # are met:
 #  * Redistributions of source code must retain the above copyright
 #    notice, this list of conditions and the following disclaimer.
 #  * Redistributions in binary form must reproduce the above copyright
 #    notice, this list of conditions and the following disclaimer in the
 #    documentation and/or other materials provided with the distribution.
 #  * Neither the name of NVIDIA CORPORATION nor the names of its
 #    contributors may be used to endorse or promote products derived
 #    from this software without specific prior written permission.
 #
 # THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS "AS IS" AND ANY
 # EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 # IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
 # PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR
 # CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 # EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 # PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
 # PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
 # OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 # (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 # OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 **************************************************************************/

/// 默认 point ReSTIR PT 的单棵路径树初采；NEE、BSDF-hit 与环境共同参与同一个 RIS。
/// NV Hybrid 与独立 RR source_pdf 契约见 replay；不改变上层配置的反弹上限。
module initial;
import math.constants;

import restir_pt.types;
import restir_pt.scene;
import restir_pt.rng;
__exported import restir_pt.initial_random;
import restir_pt.replay;
import geometry.ray;

namespace restir_pt {
/// reservoir 估计主表面之后的路径；residual 保存默认 point 域之外的主发光/主环境。
public struct InitialResult {
    public var reservoir : PathReservoir;
    public var residual : float3;
};

/// 初采追踪上下文只属于当前raygen；trace发布的新命中供下一次shade消费。
/// 已知表面的shift/sample/evaluate不经过此入口，继续由IRestirScene管理。
public interface IInitialTrace<Scene : IRestirScene, State> {
    [mutating]
    func trace(inout scene: Scene, inout state: State, origin: float3, direction: float3,
               out hit: HitInfo, out distance: float)
        ->bool;
    /// 当前新命中的表面；fromOrigin区分观察位置和已知wo，不能用于历史或光端点。
    func load(inout scene: Scene, inout state: State, hit: HitInfo, view: float3, from_origin: bool)
        ->Surface;
    [mutating]
    func shade(inout scene: Scene, inout state: State, surface: Surface, request: InitialRequest)
        ->InitialVertex;
};

/// 兼容原始TraceRay/callable实现；无额外状态或运行时分支。
public struct InitialTrace<Scene : IRestirScene> : IInitialTrace<Scene, uint> {
    [mutating]
    public func trace(inout scene: Scene, inout state: uint, origin: float3, direction: float3,
                      out hit: HitInfo, out distance: float)
        ->bool {
        var origin = origin;
        var direction = direction;
        return scene.trace(origin, direction, false, hit, distance);
    }
    public func load(inout scene: Scene, inout state: uint, hit: HitInfo, view: float3,
                     from_origin: bool)
        ->Surface {
        return from_origin ? scene.load_surface_from_origin(hit, view, false)
                           : scene.load_surface(hit, view, false);
    }
    [mutating]
    public func shade(inout scene: Scene, inout state: uint, surface: Surface,
                      request: InitialRequest)
        ->InitialVertex {
        return scene.shade_initial_vertex(surface, request);
    }
};

func initial_last_vertex(sample_value: BsdfSample, surface: Surface, rough: bool,
                         segment: ReplaySegment, primary_footprint: float,
                         distance_threshold: float)
    ->LastVertexState {
    var result : LastVertexState;
    result.init(sample_value.delta, sample_value.transmission, sample_value.component,
                surface.support_bsdf_indexing, rough,
                segment.ray_footprint >= primary_footprint * distance_threshold);
    return result;
}

func initial_can_connect(settings: ShiftSettings, depth: uint, rc_length: uint,
                         segment: ReplaySegment, outgoing_pdf: float, previous_rough: bool,
                         primary_footprint: float, distance_threshold: float)
    ->bool {

    return settings.shift_mapping == 0
               ? depth == 1
               : rc_length == MAXIMUM_RC_LENGTH && depth >= 1 && previous_rough &&
                     min(segment.ray_footprint, segment.inverse_geometry_factor / outgoing_pdf) >=
                         primary_footprint * distance_threshold;
}

/// 使用已保存 primary_hit；max_surface_bounces 须不超过254，与8bit路径长度保持一致。
/// primary_hit、初始相机/介质与当前scene须属于同一代；所有材质和求交均为实际场景操作。
/// 每个顶点合并eval/发光；预算耗尽后不采样散射，末顶点NEE不含BSDF竞争。
/// light_samples=1保留联合灯分布；=2分别采u.z的两个半区，各为一个加性灯族，
/// PDF不含族选择概率。第二族使用原padding维，不改变散射/RR及重放的随机身份。
/// 完整材质阶段一次产生可见NEE候选；调度器不重采灯、不重算材质支持或介质。
public func sample_initial_path<Scene : IRestirScene, let light_samples : uint = 1>(
    inout scene: Scene, pixel: uint2, primary_hit: HitInfo, initial_seed: uint,
    max_surface_bounces: uint, settings: ShiftSettings, use_russian_roulette: bool, use_nee: bool,
    use_mis: bool)
    ->InitialResult {
    var tracer : InitialTrace<Scene>;
    var state : uint = 0;
    return sample_initial_path_traced<Scene, uint, InitialTrace<Scene>, light_samples>(
        scene, tracer, state, pixel, primary_hit, initial_seed, max_surface_bounces, settings,
        use_russian_roulette, use_nee, use_mis);
}

/// primaryHit须由同一个tracer.trace产生；每次新命中仅在下一顶点着色前有效。
/// tracer拥有硬件命中状态，算法不复制或持久化它；其余随机域、RIS及可见性不变。
public func sample_initial_path_traced<
    Scene : IRestirScene, State, Trace : IInitialTrace<Scene, State>, let light_samples : uint = 1>(
    inout scene: Scene, inout tracer: Trace, inout state: State, pixel: uint2, primary_hit: HitInfo,
    initial_seed: uint, max_surface_bounces: uint, settings: ShiftSettings,
    use_russian_roulette: bool, use_nee: bool, use_mis: bool)
    ->InitialResult {
    static_assert(light_samples == 1 || light_samples == 2,
                  "initial NEE supports one or two families");
    var result : InitialResult = {};
    result.reservoir.init();
    if (!primary_hit.is_valid()) {
        result.residual =
            scene.environment(scene.initial_direction(pixel), scene.initial_origin(pixel), false) *
            exp(-scene.initial_medium(pixel).yzw * scene.environment_distance());
        return result;
    }
    var surface : Surface =
                      tracer.load(scene, state, primary_hit, scene.initial_origin(pixel), true);
    // primary 的物理生产者是实际相机射线；不从有限精度重建点重新推导另一条方向。
    surface.wo = -scene.initial_direction(pixel);
    scene.set_incident(surface, scene.initial_medium(pixel));
    var throughput : float3 = exp(-surface.incident.yzw * surface.distance);
    let primary_footprint = scene.primary_ray_footprint(surface, pixel, false);
    var distance_threshold : float;
    var roughness_threshold : float;
    var sampler
        : TinyUniformSampleGenerator =
              init_replay_sampler(initial_seed, settings, distance_threshold, roughness_threshold);
    var ris : PathRISState = {};
    ris.init();
    var rc : PathReconnectionInfo = {};
    rc.path_flags.insert_rc_vertex_length(MAXIMUM_RC_LENGTH);
    rc.rc_throughput = 1.0;
    rc.init_random_seed = initial_seed;
    var source_pdf = 1.0;
    var previous_scatter_pdf = 1.0;
    var previous_full_pdf = 1.0;
    var previous_rough = false;
    var previous_scatter : BsdfSample = {};
    var previous_surface : Surface = {};
    var segment : ReplaySegment = {};
    var previous_segment_transmittance : float3 = 1.0;
    for (var depth: uint = 0; depth <= max_surface_bounces; ++depth) {
        let bounce_seed = sampler.state;
        sampler.state = use_russian_roulette ? initial_random_state<11>(bounce_seed)
                                             : initial_random_state<10>(bounce_seed);
        let roulette_probability =
            use_russian_roulette ? initial_roulette_probability(throughput) : 1.0;
        let roulette_killed : bool = use_russian_roulette &&
                                     (roulette_probability == 0.0 ||
                                      initial_random_value<8>(bounce_seed) >= roulette_probability);
        let continuation = depth < max_surface_bounces;
        let request : InitialRequest = {
            bounce_seed,
            (continuation && !roulette_killed ? INITIAL_SAMPLE : 0u) |
                (use_nee ? INITIAL_NEE : 0u) | (continuation && use_mis ? INITIAL_MIS : 0u) |
                (use_russian_roulette ? INITIAL_RR : 0u) |
                (light_samples == 2 ? INITIAL_TWO_LIGHTS : 0u) |
                (depth == 0 ? INITIAL_PRIMARY : 0u),
            previous_surface.point.position,
            throughput,
            ris.weight,
            use_nee && use_mis && !previous_scatter.delta ? previous_full_pdf : -1.0
        };
        let shading : InitialVertex = tracer.shade(scene, state, surface, request);
        surface.emission = shading.emission;
        ris.weight = shading.weight;

        let last = initial_last_vertex(previous_scatter, previous_surface, previous_rough, segment,
                                       primary_footprint, distance_threshold);
        if (depth == 0) {
            result.residual = throughput * surface.emission;
        } else if (any(surface.emission > 0.0)) {
            let light_pdf = shading.emission_pdf;
            let mis = use_nee && use_mis && !previous_scatter.delta
                          ? light_mis_weight(1.0, previous_full_pdf, 1.0, light_pdf)
                          : 1.0;
            var cached_emission : float3 = surface.emission;
            if (depth - 1 == rc.path_flags.rc_vertex_length())
                cached_emission *= previous_segment_transmittance;
            ris.store_bsdf_hit_candidate(rc, depth - 1, throughput * mis * surface.emission,
                                         cached_emission, mis * surface.emission, source_pdf,
                                         light_pdf, LIGHT_EMISSIVE);
            if (rc.path_flags.rc_vertex_length() == MAXIMUM_RC_LENGTH &&
                (settings.shift_mapping == 1 || depth == 1)) {
                let can_connect : bool =
                                      settings.shift_mapping == 0 ||
                                      min(segment.ray_footprint,
                                          segment.inverse_geometry_factor * (math::tau)) >=
                                              primary_footprint * distance_threshold &&
                                          (!previous_surface.support_simple_roughness_threshold ||
                                           previous_rough);
                if (can_connect) {
                    ris.integrand *= previous_scatter_pdf;
                    ris.mark_as_rc_vertex(depth, surface.hit, last,
                                          light_pdf * segment.geometry_factor, LIGHT_EMISSIVE,
                                          surface.emission, float3(0.0), segment.geometry_factor);
                }
            }
        }
        if (shading.direct.pdf > 0.0) {
            let light : InitialDirect = shading.direct;
            let light_type : uint = light.light_type & ~INITIAL_POINT_LIGHT;
            let can_connect = initial_can_connect(settings, depth, rc.path_flags.rc_vertex_length(),
                                                  segment, light.bsdf_pdf, previous_rough,
                                                  primary_footprint, distance_threshold);
            let cached_radiance = can_connect ? light.attenuated_radiance : light.radiance;
            ris.store_nee_candidate(rc, can_connect, depth, throughput * light.reflected,
                                    light.direction, cached_radiance, light.reflected, source_pdf,
                                    light.pdf, light_type);
            ris.path_flags.insert_terminal_nee(!continuation);
            if (can_connect) {
                ris.reconnection_hit = surface.hit;
                ris.reconnection_jacobian = segment.geometry_factor;
                ris.integrand *= previous_scatter_pdf * light.pdf;
                ris.rc_next_vertex_bounce_seed = initial_random_state<2>(bounce_seed);
                ris.path_flags.try_insert_bounce_types_before_rc_vertex(last.data & 0x3fu, true);
            }
            if (!can_connect && rc.path_flags.rc_vertex_length() == MAXIMUM_RC_LENGTH) {
                ris.reconnection_hit = light.hit;
                ris.reconnection_jacobian = 1.0;
                ris.reconnection_incident_radiance = light.radiance;
                ris.reconnection_wi = light.direction;
                ris.integrand *= light.pdf;
                ris.path_flags.insert_rc_vertex_length(ris.path_flags.path_length() + 1);
                ris.path_flags.insert_is_transmission_event(
                    dot(surface.point.normal, surface.wo) *
                            dot(surface.point.normal, light.direction) <
                        0.0,
                    true);
                ris.path_flags.insert_support_bsdf_indexing_before_rc_vertex(
                    surface.support_bsdf_indexing);
                if (light.hit.is_valid()) {
                    if ((light.light_type & INITIAL_POINT_LIGHT) != 0)
                        ris.reconnection_incident_radiance /= light.geometry_factor;
                    else {
                        ris.reconnection_jacobian *= light.geometry_factor;
                        ris.light_pdf *= light.geometry_factor;
                    }
                }
            }
        }

        if (!continuation)
            break;

        if (use_russian_roulette) {
            if (roulette_killed)
                break;
            throughput /= roulette_probability;
            source_pdf *= roulette_probability;
        }
        var scatter : BsdfSample = shading.sample;
        if (scatter.pdf <= 0.0 || all(scatter.weight == 0.0) ||
            any(isnan(scatter.weight) | isinf(scatter.weight)) || isinf(scatter.pdf))
            break;
        let scatter_pdf = replay_scatter_pdf(scatter);
        let rough = !scatter.delta && scene.is_sample_rough(surface, scatter.component, scatter_pdf,
                                                            roughness_threshold);
        let component_pdf = scatter.delta                   ? 1.0
                            : surface.support_bsdf_indexing ? scatter.component_pdf
                                                            : scatter.pdf;
        throughput *= scatter.weight;
        rc.rc_throughput *= scatter.weight;
        if (initial_can_connect(settings, depth, rc.path_flags.rc_vertex_length(), segment,
                                scatter_pdf, previous_rough, primary_footprint,
                                distance_threshold)) {
            rc.rc_next_vertex_bounce_seed = sampler.state;
            rc.reconnection_wi = scatter.direction;
            rc.path_flags.insert_rc_vertex_length(depth);
            rc.reconnection_hit = surface.hit;
            rc.path_flags.try_insert_bounce_types_before_rc_vertex(last.data & 0x3fu, true);
            rc.reconnection_jacobian = segment.geometry_factor;
            source_pdf *= previous_scatter_pdf * component_pdf;
            rc.path_flags.try_insert_bounce_types_after_rc_vertex(
                uint(scatter.delta) | (uint(scatter.transmission) << 1) | (scatter.component << 2),
                true);
            rc.rc_throughput = 1.0;
        }
        let origin = geometry::offset_origin(surface.point, scatter.direction);
        var hit : HitInfo = {};
        var distance : float;
        let found : bool = tracer.trace(scene, state, origin, scatter.direction, hit, distance);
        if (!found) {
            let u_bsdfris = sampler.next_1d();
            let radiance : float3 =
                               scene.environment(scatter.direction, surface.point.position, false);
            let environment_transmittance = exp(-scatter.medium.yzw * scene.environment_distance());
            let light_pdf : float =
                                scene.light_pdf(surface, hit, scatter.direction, LIGHT_ENV, false);
            let mis = use_nee && use_mis && !scatter.delta
                          ? light_mis_weight(1.0, scatter.pdf, 1.0, light_pdf)
                          : 1.0;
            var env_last : LastVertexState;
            env_last.init(scatter.delta, scatter.transmission, scatter.component,
                          surface.support_bsdf_indexing, rough, true);
            let cached_radiance : float3 = depth == rc.path_flags.rc_vertex_length()
                                               ? radiance * environment_transmittance
                                               : radiance;
            let selected = ris.add_bsdf_hit_candidate(
                u_bsdfris, rc, depth, throughput * environment_transmittance * mis * radiance,
                scatter.direction, cached_radiance, environment_transmittance * mis * radiance,
                source_pdf, light_pdf, LIGHT_ENV);
            if (selected && rc.path_flags.rc_vertex_length() == MAXIMUM_RC_LENGTH &&
                (settings.shift_mapping == 1 || depth == 0) &&
                (rough || settings.shift_mapping == 0)) {
                ris.integrand *= component_pdf;
                ris.mark_as_rc_vertex(depth + 1, hit, env_last, light_pdf, LIGHT_ENV, radiance,
                                      scatter.direction, 1.0);
            }
            break;
        }
        let attenuation = exp(-scatter.medium.yzw * distance);
        throughput *= attenuation;
        rc.rc_throughput *= attenuation;
        previous_segment_transmittance = attenuation;
        var next : Surface = tracer.load(scene, state, hit, -scatter.direction, false);
        scene.set_incident(next, scatter.medium);
        segment = replay_segment(surface, next, scatter_pdf);
        previous_surface = surface;
        previous_scatter = scatter;
        previous_scatter_pdf = component_pdf;
        previous_full_pdf = scatter.pdf;
        previous_rough = rough;
        surface = next;
    }
    result.reservoir = PathReservoir::create_from_ris_state(ris, initial_seed);
    result.reservoir.finalize_reservoir_weight();
    return result;
}
}
