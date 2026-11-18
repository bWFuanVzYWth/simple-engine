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

/// Falcor 759aad03 默认 point Hybrid shift；原始阈值、双向可逆域与 mixed-PSS jacobian。
/// 不持有绑定或世界状态；sample/eval/trace 由场景的独立窄入口执行。
module shift;
import math.constants;

import restir_pt.types;
import restir_pt.scene;
import restir_pt.replay;
import restir_pt.storage;

namespace restir_pt {
func invalid_shift_jacobian(jacobian: float)->bool {
    return jacobian <= 0.0 || isnan(jacobian) || isinf(jacobian);
}

func invalid_shift_integrand(integrand: float3)->bool {
    return all(integrand == 0.0) || any(isnan(integrand) | isinf(integrand) | (integrand < 0.0));
}

func reject_reconnection(inout jacobian: float, inout source_geometry: float,
                         destination_geometry: float)
    ->float3 {
    jacobian = 0.0;
    source_geometry = destination_geometry;
    return 0.0;
}

/// 只执行 reconnection 两侧必要的连续 BSDF 求值与连接可见性，不采样新路径。
/// source.reconnection_jacobian 更新为目标 geometry factor，供后续 GRIS 链接。
/// 默认 additional_shadow_ray_offset=false；受控端点偏移由 scene 的精确 visibility 维护。
public func shift_path_reconnection<Scene : IRestirScene>(
    inout scene: Scene, settings: ShiftSettings, primary_ray_footprint: float,
    rc_footprint_factor: float, inout jacobian: float, dst_previous: Surface,
    inout source: PathReservoir, use_nee: bool, use_mis: bool, previous: bool)
    ->float3 {
    let rc_length = uint(source.path_flags.rc_vertex_length());
    let path_length = uint(source.path_flags.path_length());
    if (source.path_flags.decode_is_delta_event(true) ||
        source.path_flags.decode_is_delta_event(false))
        return 0.0;
    if (dst_previous.support_bsdf_indexing !=
        source.path_flags.decode_support_bsdf_indexing_before_rc_vertex())
        return 0.0;

    let rc_has_position = source.reconnection_hit.is_valid();
    let rc_is_connected_to_light = path_length == rc_length;
    let rc_is_emissive : bool = path_length + 1 == rc_length;
    let light_is_nee = source.path_flags.last_vertex_nee();
    let rc_is_nee = rc_is_connected_to_light && light_is_nee;
    let light_type = source.path_flags.light_type();
    var rc_surface : Surface = {};
    var direction : float3 = source.reconnection_wi;
    var destination_geometry = 1.0;
    var inverse_geometry = 1.0;
    var point_light = false;
    if (rc_has_position) {
        rc_surface = scene.load_surface_from_origin(source.reconnection_hit,
                                                    dst_previous.point.position, previous);
        if (!rc_surface.hit.is_valid())
            return reject_reconnection(jacobian, source.reconnection_jacobian, 0.0);
        let delta : float3 = rc_surface.point.position - dst_previous.point.position;
        let distance_squared = dot(delta, delta);
        direction = delta * rsqrt(distance_squared);
        point_light = light_type == LIGHT_ANALYTIC && rc_is_emissive &&
                      scene.is_point_light(source.reconnection_hit);
        destination_geometry =
            (point_light ? 1.0 : abs(dot(rc_surface.point.normal, -direction))) / distance_squared;
        inverse_geometry = distance_squared / abs(dot(dst_previous.point.normal, -direction));
    }
    // 这些拒绝仍须保留最终 G；上方 delta/indexing 拒绝沿用原有不写回语义。
    if (rc_has_position && rc_is_emissive && !point_light &&
        !scene.emission_side_supported(rc_surface))
        return reject_reconnection(jacobian, source.reconnection_jacobian, destination_geometry);

    var distance_threshold : float;
    var roughness_threshold : float;
    init_replay_sampler(source.init_random_seed, settings, distance_threshold, roughness_threshold);
    let first_component = dst_previous.support_bsdf_indexing && !(rc_is_emissive && light_is_nee)
                              ? int(source.path_flags.decode_bsdf_component(true))
                              : -1;
    let connection : ConnectionEvaluation = scene.evaluate_connection(
                         dst_previous, direction,
                         rc_has_position ? rc_surface.distance : scene.environment_distance(),
                         first_component);
    let first : BsdfEvaluation = connection.bsdf;
    // rcPrevious 在目标前缀中不能已成为更早的 reconnection vertex。
    if (rc_length > 1 && rc_footprint_factor > 0.0 &&
        rc_footprint_factor / first.pdf >= primary_ray_footprint * distance_threshold)
        return reject_reconnection(jacobian, source.reconnection_jacobian, destination_geometry);
    let check_reverse_threshold
        : bool = settings.shift_mapping == 1 && !(rc_is_emissive && light_is_nee);
    if (check_reverse_threshold) {
        let violates_roughness =
            !(rc_is_emissive && !light_is_nee &&
              !dst_previous.support_simple_roughness_threshold) &&
            !scene.is_sample_rough(dst_previous, source.path_flags.decode_bsdf_component(true),
                                   first.pdf, roughness_threshold);
        let ray_footprint : float = 1.0 / (first.pdf * destination_geometry);
        let violates_distance =
            rc_has_position && ray_footprint < primary_ray_footprint * distance_threshold;
        if (violates_roughness || violates_distance)
            return reject_reconnection(jacobian, source.reconnection_jacobian,
                                       destination_geometry);
    }
    let incident_radiance
        : float3 = rc_is_emissive && !rc_has_position && light_type == LIGHT_ENV
                       ? scene.environment(direction, dst_previous.point.position, previous)
                       : source.reconnection_incident_radiance;
    var integrand : float3 = incident_radiance * first.value;
    integrand *= connection.transmittance;
    if (rc_is_emissive && use_nee && use_mis && light_type != LIGHT_ANALYTIC &&
        !(light_is_nee && source.path_flags.terminal_nee())) {
        var light_pdf : float = source.light_pdf / destination_geometry;
        if ((rc_has_position && light_type == LIGHT_EMISSIVE) ||
            (!rc_has_position && light_type == LIGHT_ENV))
            light_pdf = scene.light_pdf(dst_previous, source.reconnection_hit, direction,
                                        light_type, previous);
        integrand *= light_is_nee ? light_mis_weight(1.0, light_pdf, 1.0, first.pdf)
                                  : light_mis_weight(1.0, first.pdf, 1.0, light_pdf);
    }

    let need_second = rc_has_position && !rc_is_emissive;
    var second_pdf : float = 0.5 / math::pi;
    if (need_second) {
        rc_surface.incident.x = connection.incident_ior;
        let second_component = rc_surface.support_bsdf_indexing && !rc_is_nee
                                   ? int(source.path_flags.decode_bsdf_component(false))
                                   : -1;
        let second : BsdfEvaluation =
                         scene.evaluate_bsdf(rc_surface, source.reconnection_wi, second_component);
        integrand *= second.value;
        second_pdf = second.pdf;
        if (rc_is_connected_to_light && use_nee && use_mis && light_type != LIGHT_ANALYTIC &&
            !(rc_is_nee && source.path_flags.terminal_nee()))
            integrand *= rc_is_nee ? light_mis_weight(1.0, source.light_pdf, 1.0, second_pdf)
                                   : light_mis_weight(1.0, second_pdf, 1.0, source.light_pdf);
    }
    // emission-BSDF 端点使用 NV 的 diffuse pseudo PDF；NEE 强制光源重连不做反向阈值。
    if (check_reverse_threshold && !(rc_is_emissive && !rc_has_position) &&
        inverse_geometry / second_pdf < primary_ray_footprint * distance_threshold)
        return reject_reconnection(jacobian, source.reconnection_jacobian, destination_geometry);
    jacobian *= destination_geometry / source.reconnection_jacobian;
    // 负值只在所有 BSDF/MIS 乘积完成后判定，避免提前拒绝随后会被零消去的分量。
    if (invalid_shift_jacobian(jacobian) || invalid_shift_integrand(integrand))
        return reject_reconnection(jacobian, source.reconnection_jacobian, destination_geometry);
    let visible = rc_has_position ? scene.visible(dst_previous, rc_surface, previous)
                                  : scene.visible_direction(dst_previous, direction, previous);
    if (!visible)
        return reject_reconnection(jacobian, source.reconnection_jacobian, destination_geometry);
    source.reconnection_jacobian = destination_geometry;
    return integrand;
}

/// 使用已完成的 prefix 重放结果；空 rc_prev_hit 只允许无 reconnection 的完整 PSS 路径。
public func shift_path_from_replay<Scene : IRestirScene>(
    inout scene: Scene, settings: ShiftSettings, primary_ray_footprint: float,
    replay: ReconnectionData, inout source: PathReservoir, use_nee: bool, use_mis: bool,
    previous: bool, out jacobian: float)
    ->float3 {
    jacobian = 1.0;
    var throughput : float3 = source.weight != 0.0 ? replay.path_throughput : float3(0.0);
    if (any(throughput > 0.0) && replay.rc_prev_hit.is_valid()) {
        var previous_surface
            : Surface = scene.load_surface(replay.rc_prev_hit, replay.rc_prev_wo, previous);
        if (!previous_surface.hit.is_valid()) {
            jacobian = 0.0;
            return 0.0;
        }
        scene.set_incident(previous_surface, replay.rc_prev_incident);
        throughput *= shift_path_reconnection(scene, settings, primary_ray_footprint,
                                              replay.rc_footprint_factor, jacobian,
                                              previous_surface, source, use_nee, use_mis, previous);
    } else if (source.path_flags.rc_vertex_length() <= source.path_flags.path_length() + 1) {
        throughput = 0.0;
    }
    return throughput;
}

/// 只读源在本轮保持不变；prefix只载入12B热字段，冷身份在其结束后恢复。
/// 这样replay中的材质callable/SER无需保存整份80B源；所有端点/介质仍由场景精确恢复。
/// 两源入口只在加载边界选择A/B，replay/shift体保持单份，避免temporal方向复制整个RT控制流。
/// historySource只选源buffer；previous只选目标几何/光照世代，二者语义独立。
/// buffer与index引用固定本轮A或已接受历史B，不在本函数提交、交换或修改源。
public func shift_path_loaded<Scene : IRestirScene>(
    inout scene: Scene, primary: Surface, current: RWByteAddressBuffer,
    history: RWByteAddressBuffer, index: uint, history_source: bool, primary_ray_footprint: float,
    settings: ShiftSettings, use_russian_roulette: bool, use_nee: bool, use_mis: bool,
    previous: bool)
    ->PrecomputedShiftData {
    var source : PathReservoir = history_source ? load_replay_source(history, index)
                                                : load_replay_source(current, index);
    let replay = trace_replayed_path(scene, primary, source, primary_ray_footprint, settings,
                                     use_russian_roulette, use_nee, use_mis, previous);
    source = history_source ? load_reservoir(history, index) : load_reservoir(current, index);
    var result : PrecomputedShiftData = {};
    result.path_throughput =
        shift_path_from_replay(scene, settings, primary_ray_footprint, replay, source, use_nee,
                               use_mis, previous, result.jacobian);
    result.reconnection_jacobian = source.reconnection_jacobian;
    return result;
}

/// 单一只读源的入口；复用同一冷热恢复契约，编译器消除恒定false的历史选择。
public func shift_path_loaded<Scene : IRestirScene>(inout scene: Scene, primary: Surface,
                                                    buffer: RWByteAddressBuffer, index: uint,
                                                    primary_ray_footprint: float,
                                                    settings: ShiftSettings,
                                                    use_russian_roulette: bool, use_nee: bool,
                                                    use_mis: bool, previous: bool)
    ->PrecomputedShiftData {
    return shift_path_loaded(scene, primary, buffer, buffer, index, false, primary_ray_footprint,
                             settings, use_russian_roulette, use_nee, use_mis, previous);
}

/// 完整的 point shift；逆向 pairwise MIS 必须独立以反向 primary 再调用一次，不能反用已改写的
/// source。
public func shift_path<Scene : IRestirScene>(inout scene: Scene, primary: Surface,
                                             inout source: PathReservoir,
                                             primary_ray_footprint: float, settings: ShiftSettings,
                                             use_russian_roulette: bool, use_nee: bool,
                                             use_mis: bool, previous: bool, out jacobian: float)
    ->float3 {
    let replay = trace_replayed_path(scene, primary, source, primary_ray_footprint, settings,
                                     use_russian_roulette, use_nee, use_mis, previous);
    return shift_path_from_replay(scene, settings, primary_ray_footprint, replay, source, use_nee,
                                  use_mis, previous, jacobian);
}
}
