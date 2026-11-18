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

/// Falcor 759aad03 默认 point Hybrid 的前缀重放；场景和材质操作由 IRestirScene 注入。
/// 初采和重放共同使用本模块的随机消费契约；重放不执行 Russian roulette。
module replay;
import math.constants;

import restir_pt.types;
import restir_pt.rng;
import restir_pt.scene;
import geometry.ray;

namespace restir_pt {
/// 当前 bounce 的随机输入；light 的后两维是 NV 非 LightTileRIS 路径的固定 padding。
public struct ReplayRandom {
    public var u_bsdfris : float;
    public var u_neeris : float;
    public var nee_seed : uint;
    public var light_selector : float;
    public var light : float4;
    public var roulette : float;
    /// xy 为方向，z 为 lobe/Fresnel 选择；RNG 中仍先消费选择维。
    public var bsdf : float3;
};

/// 恢复源路径种子并消费阈值；initial 也必须调用此入口，不另建阈值随机流。
public func init_replay_sampler(seed: uint, settings: ShiftSettings, out distance_threshold: float,
                                out roughness_threshold: float)
    ->TinyUniformSampleGenerator {
    var sampler : TinyUniformSampleGenerator = rng_from_seed(seed);
    distance_threshold = settings.near_field_distance_threshold;
    roughness_threshold = settings.specular_roughness_threshold;
    if (settings.near_field_distance_sigma != 0.0)
        distance_threshold = PathRISState::gaussrand(
            distance_threshold, settings.near_field_distance_sigma, sampler.next_2d());
    if (settings.specular_roughness_sigma != 0.0)
        roughness_threshold = PathRISState::gaussrand(
            roughness_threshold, settings.specular_roughness_sigma, sampler.next_2d());
    return sampler;
}

/// 非 LightTileRIS 的共同消费顺序；关闭 NEE 或碰到 delta 时仍消费光源 padding。
/// RR 开启消费 11 维、关闭消费 10 维；只有 initial 使用 roulette 的存活决策。
public func bounce_random(inout sampler: TinyUniformSampleGenerator, use_russian_roulette: bool)
    ->ReplayRandom {
    var result : ReplayRandom;
    result.u_bsdfris = sampler.next_1d();
    result.u_neeris = sampler.next_1d();
    result.nee_seed = sampler.state;
    result.light_selector = sampler.next_1d();
    result.light = sampler.next4_d();
    result.roulette = use_russian_roulette ? sampler.next_1d() : 0.0;
    let selector = sampler.next_1d();
    result.bsdf = float3(sampler.next_2d(), selector);
    return result;
}

/// 场景光源适配器使用联合 CDF；z 选择策略后须重映射为该策略内的均匀残差。
public func light_random(random: ReplayRandom)->float3 {
    return float3(random.light.xy, random.light_selector);
}

/// NV 默认 Balance MIS；参数已经含各策略的样本数，不混用旧 PT 的 power heuristic。
public func light_mis_weight(n0: float, p0: float, n1: float, p1: float)->float {
    let first = n0 * p0;
    let second = n1 * p1;
    return first > 0.0 ? first / (first + second) : 0.0;
}

/// 初采外部 RR 的存活率；存活时 throughput 除此值，source_pdf 乘此值。
/// mixed-PSS 的重放去掉 roulette 维，仅跳过相同随机数，不再次补偿吞吐。
public func initial_roulette_probability(throughput: float3)->float {
    return min(1.0, max(0.0, PathRISState::to_scalar(throughput)));
}

/// 连续 BSDF 用立体角密度，delta 仅在 footprint 中用 NV 的有限大替代值。
public func replay_scatter_pdf(sample_value: BsdfSample)->float {
    return sample_value.delta ? 1.0e20 : sample_value.pdf;
}

/// 每段的几何量；两端点均为未偏移表面位置，visibility 的偏移不进入积分 measure。
public struct ReplaySegment {
    public var geometry_factor : float;
    public var inverse_geometry_factor : float;
    public var ray_footprint : float;
};

/// 与 initial 的 setRcVertex 和 NEE/emitter 特例共同使用。
public func replay_segment(previous: Surface, current: Surface, scatter_pdf: float)->ReplaySegment {
    let delta : float3 = current.point.position - previous.point.position;
    let distance_squared = dot(delta, delta);
    let wo = -delta * rsqrt(distance_squared);
    var result : ReplaySegment;
    result.geometry_factor = abs(dot(current.point.normal, wo)) / distance_squared;
    result.inverse_geometry_factor = distance_squared / abs(dot(previous.point.normal, wo));
    result.ray_footprint = 1.0 / (result.geometry_factor * scatter_pdf);
    return result;
}

/// rc_vertex_length=1 时直接使用目标 primary；其余路径只采样 prefix，不调用完整材质求值。
/// 无 reconnection 顶点（长度 255）时重放整个选中 BSDF-hit/environment 路径，返回真实目标
/// integrand。 path_length 是最后 scatter 的零基索引；primary scatter 为
/// 0。无效或不可逆路径返回零贡献。
public func trace_replayed_path<Scene : IRestirScene>(inout scene: Scene, primary: Surface,
                                                      source: PathReservoir,
                                                      primary_ray_footprint: float,
                                                      settings: ShiftSettings,
                                                      use_russian_roulette: bool, use_nee: bool,
                                                      use_mis: bool, previous: bool)
    ->ReconnectionData {
    var result : ReconnectionData = {};
    if (!primary.hit.is_valid() || source.weight == 0.0)
        return result;
    let rc_length = uint(source.path_flags.rc_vertex_length());
    let path_length = uint(source.path_flags.path_length());
    let should_connect : bool = rc_length <= path_length + 1;
    let primary_throughput = exp(-primary.incident.yzw * primary.distance);
    if (should_connect && rc_length == 1) {
        if (!primary.has_non_delta_lobes)
            return result;
        result.rc_prev_hit = primary.hit;
        result.rc_prev_wo = primary.wo;
        result.rc_prev_incident = primary.incident;
        result.path_throughput = primary_throughput;
        result.rc_footprint_factor = -1.0;
        return result;
    }
    if (source.path_flags.last_vertex_nee() && !should_connect)
        return result;

    var distance_threshold : float;
    var roughness_threshold : float;
    var sampler : TinyUniformSampleGenerator = init_replay_sampler(
                      source.init_random_seed, settings, distance_threshold, roughness_threshold);
    var surface : Surface = primary;
    var throughput : float3 = primary_throughput;
    var last_rough = false;
    var incoming : ReplaySegment = {};
    for (var depth: uint = 0; depth <= path_length; ++depth) {
        let random = bounce_random(sampler, use_russian_roulette);
        var scatter : BsdfSample = scene.sample_bsdf(surface, random.bsdf);
        if (scatter.pdf <= 0.0 || all(scatter.weight == 0.0) ||
            any(isnan(scatter.weight) | isinf(scatter.weight)) || isinf(scatter.pdf))
            return result;
        let scatter_pdf = replay_scatter_pdf(scatter);
        let rough = !scatter.delta && scene.is_sample_rough(surface, scatter.component, scatter_pdf,
                                                            roughness_threshold);

        // 在到达源 reconnection 前不能出现更早可连接顶点，否则映射不能逆回同一 prefix。
        if (depth > 0 && depth < rc_length && last_rough &&
            min(incoming.ray_footprint, incoming.inverse_geometry_factor / scatter_pdf) >=
                primary_ray_footprint * distance_threshold)
            return result;

        throughput *= scatter.weight;
        var origin : float3 = geometry::offset_origin(surface.point, scatter.direction);
        var hit : HitInfo = {};
        var distance : float;
        let found : bool = scene.trace(origin, scatter.direction, previous, hit, distance);
        if (!found) {
            if (should_connect || depth != path_length ||
                source.path_flags.light_type() != LIGHT_ENV || rough)
                return result;
            var radiance
                : float3 = scene.environment(scatter.direction, surface.point.position, previous);
            radiance *= exp(-scatter.medium.yzw * scene.environment_distance());
            let light_pdf
                : float = scene.light_pdf(surface, hit, scatter.direction, LIGHT_ENV, previous);
            let mis = use_nee && use_mis && !scatter.delta
                          ? light_mis_weight(1.0, scatter.pdf, 1.0, light_pdf)
                          : 1.0;
            result.path_throughput = throughput * radiance * mis;
            return result;
        }

        throughput *= exp(-scatter.medium.yzw * distance);
        let next_depth : uint = depth + 1;
        if (next_depth == path_length + 1 && !source.path_flags.last_vertex_nee()) {
            if (should_connect || source.path_flags.light_type() != LIGHT_EMISSIVE)
                return result;
            var endpoint : Surface = {};
            endpoint.hit = hit;
            endpoint.wo = -scatter.direction;
            let emitter : EmitterEvaluation =
                              scene.evaluate_emitter(endpoint, surface.point.position, previous);
            if (!any(emitter.emission > 0.0))
                return result;
            endpoint.point.position = emitter.position;
            endpoint.point.normal = emitter.normal;
            let emitter_segment = replay_segment(surface, endpoint, scatter_pdf);
            let emitter_could_reconnect
                : bool = min(emitter_segment.ray_footprint,
                             emitter_segment.inverse_geometry_factor * (math::tau)) >=
                             primary_ray_footprint * distance_threshold &&
                         (!surface.support_simple_roughness_threshold || rough);
            if (next_depth < rc_length && emitter_could_reconnect)
                return result;
            let light_pdf = emitter.pdf;
            let mis = use_nee && use_mis && !scatter.delta
                          ? light_mis_weight(1.0, scatter.pdf, 1.0, light_pdf)
                          : 1.0;
            result.path_throughput = throughput * emitter.emission * mis;
            return result;
        }
        var next : Surface = scene.load_surface(hit, -scatter.direction, previous);
        scene.set_incident(next, scatter.medium);
        incoming = replay_segment(surface, next, scatter_pdf);
        if (should_connect && next_depth == rc_length - 1) {
            if (!next.has_non_delta_lobes)
                return result;
            result.rc_prev_hit = hit;
            result.rc_prev_wo = next.wo;
            result.rc_prev_incident = next.incident;
            result.path_throughput = throughput;
            result.rc_footprint_factor =
                !rough || incoming.ray_footprint < primary_ray_footprint * distance_threshold
                    ? -1.0
                    : incoming.inverse_geometry_factor;
            return result;
        }
        surface = next;
        last_rough = rough;
    }
    return result;
}
}
