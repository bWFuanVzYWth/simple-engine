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
 # PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER OR
 # CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 # EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 # PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
 # PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
 # OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 # (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 # OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 **************************************************************************/

/// Falcor 759aad03 默认 point ReSTIR PT 的未压缩中立ABI；scalar布局为80/60/20B。
/// identity的全0保留为无效，scene负责其余uint4编码、稳定性与重建，不保存宿主指针。
module types;
import math.constants;

namespace restir_pt
{
/// NV PathTracerUtils::LightType的2-bit编码。
public static const let LIGHT_ENV : uint = 0;
public static const let LIGHT_EMISSIVE : uint = 1;
public static const let LIGHT_ANALYTIC : uint = 2;
public static const let MAXIMUM_RC_LENGTH : uint = 255;

/// 默认point Hybrid设置；distance阈值已按NV的c/100换算。
public struct ShiftSettings
{
    /// 0为纯reconnection，1为Hybrid random replay/reconnection。
    public var shift_mapping : uint = 1;
    public var local_strategy_type : uint = 3;
    public var specular_roughness_threshold = 0.2f;
    public var near_field_distance_threshold = 0.0002f;
    public var specular_roughness_sigma = 0.0f;
    public var near_field_distance_sigma = 0.2f;
    public var additional_shadow_ray_offset = false;
};

/// 16B稳定交点身份；几何、材质、重心的解释归scene适配器。
public struct HitInfo
{
    /// 全0表示无效；有效身份须至少有一个非零分量。
    public var data : uint4 = 0;
    /// 是否有有效身份，不访问scene资源。
    public func is_valid()->bool
    {
        return any(data != 0);
    }
    /// 与isValid等价的简短入口。
    public func valid()->bool
    {
        return is_valid();
    }
};

/// NV原位布局：0..7为path length，8..15为rc length，16为NEE，17为primary direct，
/// 18..19为lightType，20..25为rc前delta/transmission/component/indexing，
/// 26..30为rc后delta/transmission/component，31为末顶点NEE无BSDF竞争；
/// 255是无可重连顶点的rc sentinel。
public struct ReSTIRPathFlags
{
    public static const let k_path_length_mask : uint = 0xFF;
    public static const let k_path_length_shift : uint = 0;
    public static const let k_rc_vertex_length_mask : uint = 0xFF;
    public static const let k_rc_vertex_length_shift : uint = 8;
    public static const let k_last_vertex_nee_shift : uint = 16;
    public static const let k_is_very_direct_light_shift : uint = 17;
    public static const let k_light_type_mask : uint = 0x3;
    public static const let k_light_type_shift : uint = 18;

    public static const let k_bounce_packed_before_rc_mask : uint = 0x3f;
    public static const let k_bounce_packed_before_rc_shift : uint = 20;
    public static const let k_is_delta_before_rc_shift : uint = 20;
    public static const let k_is_transmission_before_rc_shift : uint = 21;
    public static const let k_bsdf_component_before_rc_shift : uint = 22;
    public static const let k_support_bsdf_indexing_shift : uint = 25;

    public static const let k_bounce_packed_after_rc_mask : uint = 0x1f;
    public static const let k_bounce_packed_after_rc_shift : uint = 26;
    public static const let k_is_delta_after_rc_shift : uint = 26;
    public static const let k_is_transmission_after_rc_shift : uint = 27;
    public static const let k_bsdf_component_after_rc_shift : uint = 28;

    public static const let k_bsdf_component_mask : uint = 0x7;
    public static const let k_terminal_nee_shift : uint = 31;

    public var flags : uint = 0;

    [mutating]
    public func try_insert_bounce_types_before_rc_vertex(data: uint, do_insert: bool)->void
    {
        flags &= do_insert ? ~(k_bounce_packed_before_rc_mask << k_bounce_packed_before_rc_shift)
                           : 0xffffffff;
        flags |= do_insert ? (data << k_bounce_packed_before_rc_shift) : 0;
    }

    [mutating]
    public func try_insert_bounce_types_after_rc_vertex(data: uint, do_insert: bool)->void
    {
        flags &= do_insert ? ~(k_bounce_packed_after_rc_mask << k_bounce_packed_after_rc_shift)
                           : 0xffffffff;
        flags |= do_insert ? (data << k_bounce_packed_after_rc_shift) : 0;
    }

    [mutating]
    public func insert_is_delta_event(is_delta_event: bool, before_rc_vertex: bool)->void
    {
        let shift : uint =
                        before_rc_vertex ? k_is_delta_before_rc_shift : k_is_delta_after_rc_shift;
        flags &= ~(1 << shift);
        flags |= uint(is_delta_event) << shift;
    }

    [mutating]
    public func insert_is_transmission_event(is_transmission_event: bool, before_rc_vertex: bool)
        ->void
    {
        let shift : uint = before_rc_vertex ? k_is_transmission_before_rc_shift
                                            : k_is_transmission_after_rc_shift;
        flags &= ~(1 << shift);
        flags |= uint(is_transmission_event) << shift;
    }

    [mutating]
    public func insert_bsdf_component_type(component: uint, before_rc_vertex: bool)->void
    {
        let shift : uint = before_rc_vertex ? k_bsdf_component_before_rc_shift
                                            : k_bsdf_component_after_rc_shift;
        flags &= ~(k_bsdf_component_mask << shift);
        flags |= component << shift;
    }

    [mutating]
    public func insert_support_bsdf_indexing_before_rc_vertex(enabled: bool)->void
    {
        flags &= ~(1 << k_support_bsdf_indexing_shift);
        flags |= uint(enabled) << k_support_bsdf_indexing_shift;
    }

    public func decode_is_delta_event(before_rc_vertex: bool)->bool
    {
        return ((flags >>
                 (before_rc_vertex ? k_is_delta_before_rc_shift : k_is_delta_after_rc_shift)) &
                1) != 0u;
    }

    public func decode_is_transmission_event(before_rc_vertex: bool)->bool
    {
        return ((flags >> (before_rc_vertex ? k_is_transmission_before_rc_shift
                                            : k_is_transmission_after_rc_shift)) &
                1) != 0u;
    }

    public func decode_bsdf_component(before_rc_vertex: bool)->uint
    {
        return (flags >> (before_rc_vertex ? k_bsdf_component_before_rc_shift
                                           : k_bsdf_component_after_rc_shift)) &
               k_bsdf_component_mask;
    }

    public func decode_support_bsdf_indexing_before_rc_vertex()->bool
    {
        return ((flags >> k_support_bsdf_indexing_shift) & 1) != 0u;
    }

    [mutating]
    public func insert_path_length(path_length: int)->void
    {
        flags &= ~k_path_length_mask;
        flags |= path_length & k_path_length_mask;
    }

    [mutating]
    public func insert_rc_vertex_length(rc_vertex_length: int)->void
    {
        flags &= ~(k_rc_vertex_length_mask << k_rc_vertex_length_shift);
        flags |= (rc_vertex_length & k_rc_vertex_length_mask) << k_rc_vertex_length_shift;
    }

    [mutating]
    public func reset_rc_vertex_length()->void
    {
        flags |= k_rc_vertex_length_mask << k_rc_vertex_length_shift;
    }

    public func path_length()->int
    {
        return flags & k_path_length_mask;
    }

    public func rc_vertex_length()->int
    {
        return (flags >> k_rc_vertex_length_shift) & k_rc_vertex_length_mask;
    }

    [mutating]
    public func insert_last_vertex_nee(is_nee: bool)->void
    {
        flags &= ~(1 << k_last_vertex_nee_shift);
        flags |= (int(is_nee) & 1) << k_last_vertex_nee_shift;
    }

    public func last_vertex_nee()->bool
    {
        return ((flags >> k_last_vertex_nee_shift) & 1) != 0u;
    }

    /// 散射预算耗尽后的NEE没有BSDF采样竞争；随赢家保存，预算热切不能改写旧样本。
    [mutating]
    public func insert_terminal_nee(terminal: bool)->void
    {
        flags = (flags & ~(1u << k_terminal_nee_shift)) | (uint(terminal) << k_terminal_nee_shift);
    }

    public func terminal_nee()->bool
    {
        return (flags >> k_terminal_nee_shift) != 0u;
    }

    [mutating]
    public func insert_is_very_direct_light(is_very_direct_light: bool)->void
    {
        flags &= ~(1 << k_is_very_direct_light_shift);
        flags |= (int(is_very_direct_light) & 1) << k_is_very_direct_light_shift;
    }

    public func is_very_direct_light()->bool
    {
        return ((flags >> k_is_very_direct_light_shift) & 1) != 0u;
    }

    [mutating]
    public func insert_light_type(light_type: uint)->void
    {
        flags &= ~(k_light_type_mask << k_light_type_shift);
        flags |= ((int(light_type) & k_light_type_mask) << k_light_type_shift);
    }

    public func light_type()->uint
    {
        return (flags >> k_light_type_shift) & k_light_type_mask;
    }
};

public struct LastVertexState
{
    public static const let k_is_delta_shift : uint = 0;
    public static const let k_is_transmission_shift : uint = 1;
    public static const let k_bsdf_component_shift : uint = 2;
    public static const let k_bsdf_component_mask : uint = 0x7;
    public static const let k_support_bsdf_indexing_shift : uint = 5;
    public static const let k_is_rough_shift : uint = 6;
    public static const let k_is_far_from_prev_shift : uint = 7;

    public var data : uint = 0;

    [mutating]
    public func init(is_last_vertex_delta: bool, is_last_vertex_transmission: bool,
                     last_bsdf_component: uint, last_vertex_support_bsdf_indexing: bool,
                     is_last_vertex_rough: bool, is_current_vertex_far_from_prev: bool)
        ->void
    {
        data = 0;
        data |= (uint)is_last_vertex_delta << k_is_delta_shift;
        data |= (uint)is_last_vertex_transmission << k_is_transmission_shift;
        data |= (uint)last_bsdf_component << k_bsdf_component_shift;
        data |= (uint)last_vertex_support_bsdf_indexing << k_support_bsdf_indexing_shift;
        data |= (uint)is_last_vertex_rough << k_is_rough_shift;
        data |= (uint)is_current_vertex_far_from_prev << k_is_far_from_prev_shift;
    }

    public func is_last_vertex_delta()->bool
    {
        return ((data >> k_is_delta_shift) & 1) != 0u;
    }
    public func is_last_vertex_transmission()->bool
    {
        return ((data >> k_is_transmission_shift) & 1) != 0u;
    }
    public func last_bsdf_component()->uint
    {
        return (data >> k_bsdf_component_shift) & k_bsdf_component_mask;
    }
    public func last_vertex_support_bsdf_indexing()->bool
    {
        return ((data >> k_support_bsdf_indexing_shift) & 1) != 0u;
    }
    public func is_last_vertex_rough()->bool
    {
        return ((data >> k_is_rough_shift) & 1) != 0u;
    }
    public func is_current_vertex_far_from_prev()->bool
    {
        return ((data >> k_is_far_from_prev_shift) & 1) != 0u;
    }
}

/// 只在当前追踪中使用的rc前缀状态；不是跨帧buffer ABI。
public struct PathReconnectionInfo
{
    public var path_flags : ReSTIRPathFlags = {};
    public var rc_next_vertex_bounce_seed : uint = 0;
    public var init_random_seed : uint = 0;
    public var rc_throughput : float3 = 0;
    public var reconnection_hit : HitInfo = {};
    public var reconnection_jacobian : float = 0;
    public var reconnection_wi : float3 = 0;
    public var footprint_factor : float = 0;
}

/// 路径内部候选RIS，weight为未归一化和；混合PSS/solid-angle integrand由scene形成。
public struct PathRISState
{
    public var weight : float = 0;
    public var integrand : float3 = 0;
    public var path_flags : ReSTIRPathFlags = {};
    public var light_pdf : float = 0;
    public var rc_next_vertex_bounce_seed : uint = 0;
    public var reconnection_hit : HitInfo = {};
    public var reconnection_jacobian : float = 0;
    public var reconnection_incident_radiance : float3 = 0;
    public var reconnection_wi : float3 = 0;

    [mutating]
    public func init()->void
    {
        weight = 0.f;
    }

    public static func gaussrand(mean: float, stddev: float, random_uv: float2)->float
    {
        let rand = random_uv;
        let normal_deviate = sqrt(-2.0 * log(rand.x)) * cos(math::tau * rand.y);
        let threshold = normal_deviate * stddev * mean + mean;
        return threshold;
    }

    /// 叶子已按发光、局部灯、环境顺序抽取胜者。此处仅写胜者的重连属性。
    [mutating]
    public func store_nee_candidate(rc: PathReconnectionInfo, is_rc_vertex: bool, depth: uint,
                                    contribution: float3, wi: float3, Le: float3, Lr: float3,
                                    source_pdf: float, pdf: float, light_type: uint)
        ->void
    {
        path_flags = rc.path_flags;
        path_flags.insert_path_length(depth);
        if (is_rc_vertex)
            path_flags.insert_rc_vertex_length(depth);
        path_flags.insert_last_vertex_nee(true);
        path_flags.insert_light_type(light_type);
        reconnection_incident_radiance = is_rc_vertex ? Le : rc.rc_throughput * Lr;
        reconnection_wi = is_rc_vertex ? wi : rc.reconnection_wi;
        reconnection_hit = rc.reconnection_hit;
        reconnection_jacobian = rc.reconnection_jacobian;
        rc_next_vertex_bounce_seed = rc.rc_next_vertex_bounce_seed;
        integrand = contribution * source_pdf;
        light_pdf = pdf;
    }

    [mutating]
    public func store_bsdf_hit_candidate(rc: PathReconnectionInfo, depth: uint,
                                         contribution: float3, Le: float3, Lr: float3,
                                         source_pdf: float, pdf: float, light_type: uint)
        ->void
    {
        path_flags = rc.path_flags;
        path_flags.insert_path_length(depth);
        path_flags.insert_last_vertex_nee(false);
        path_flags.insert_light_type(light_type);
        reconnection_incident_radiance =
            depth == rc.path_flags.rc_vertex_length() ? Le : rc.rc_throughput * Lr;
        reconnection_wi = rc.reconnection_wi;
        reconnection_hit = rc.reconnection_hit;
        reconnection_jacobian = rc.reconnection_jacobian;
        rc_next_vertex_bounce_seed = rc.rc_next_vertex_bounce_seed;
        integrand = contribution * source_pdf;
        light_pdf = pdf;
    }

    [mutating]
    public func add_bsdf_hit_candidate(u: float, rc: PathReconnectionInfo, depth: uint,
                                       contribution: float3, wi: float3, Le: float3, Lr: float3,
                                       source_pdf: float, pdf: float, light_type: uint)
        ->bool
    {
        if (!add_pss_sample(u, contribution))
            return false;
        store_bsdf_hit_candidate(rc, depth, contribution, Le, Lr, source_pdf, pdf, light_type);
        return true;
    }

    [mutating]
    public func add_primary_visible_emitter_candidate(u: float, path_contribution: float3,
                                                      source_pdf: float)
        ->bool
    {
        let selected = add_pss_sample(u, path_contribution);
        this.path_flags.flags = selected ? 0 : this.path_flags.flags;
        this.path_flags.insert_is_very_direct_light(
            selected ? true : this.path_flags.is_very_direct_light());

        this.integrand = selected ? path_contribution * source_pdf : this.integrand;
        return selected;
    }

    [mutating]
    public func mark_as_rc_vertex(path_length: uint, hit: HitInfo,
                                  last_vertex_state: LastVertexState, light_pdf: float,
                                  light_type: uint, Le: float3, wi: float3,
                                  reconnection_jacobian: float)
        ->void
    {
        this.path_flags.insert_rc_vertex_length(path_length);

        this.reconnection_hit = hit;
        this.path_flags.insert_is_delta_event(last_vertex_state.is_last_vertex_delta(), true);
        this.path_flags.insert_is_transmission_event(
            last_vertex_state.is_last_vertex_transmission(), true);
        this.path_flags.insert_bsdf_component_type(last_vertex_state.last_bsdf_component(), true);
        this.path_flags.insert_support_bsdf_indexing_before_rc_vertex(
            last_vertex_state.last_vertex_support_bsdf_indexing());
        this.light_pdf = light_pdf;

        this.reconnection_jacobian = reconnection_jacobian;

        this.path_flags.insert_light_type(light_type);
        this.reconnection_incident_radiance = Le;
        this.reconnection_wi = wi;
    }

    [mutating]
    public func add_pss_sample(u: float, path_contribution: float3)->bool
    {
        return select_candidate(weight, u, path_contribution);
    }

    public static func select_candidate(inout total: float, u: float, path_contribution: float3)
        ->bool
    {
        var w : float = to_scalar(path_contribution);

        w = isnan(w) ? 0.0f : w;
        total += w;

        return (u * total < w);
    }

    public static func to_scalar(color: float3)->float
    {
        return dot(color, float3(0.299, 0.587, 0.114));
    }
}

/// 单像素canonical reservoir；weight构建时为和，finalize后为W。
/// M为候选/历史有效量，seed及rc后随机种子用于确定性重放。
public struct PathReservoir
{
    public var sample_mass : float;
    public var weight : float;
    public var integrand : float3;
    public var path_flags : ReSTIRPathFlags;
    public var init_random_seed : uint;
    public var rc_next_vertex_bounce_seed : uint;
    public var light_pdf : float;
    public var reconnection_hit : HitInfo;
    public var reconnection_jacobian : float;
    public var reconnection_incident_radiance : float3;
    public var reconnection_wi : float3;

    [mutating]
    public func init()->void
    {
        this.sample_mass = 0;
        this.weight = 0;
        this.integrand = 0;
        this.path_flags.flags = 0;
        this.init_random_seed = 0;
        this.rc_next_vertex_bounce_seed = 0;
        this.light_pdf = 0;
        this.reconnection_hit.data = 0;
        this.reconnection_jacobian = 0;
        this.reconnection_incident_radiance = 0;
        this.reconnection_wi = 0;
    }

    [mutating]
    public func init(const state: PathRISState)->void
    {
        this.sample_mass = 1;
        this.weight = state.weight;
        this.integrand = state.integrand;
        this.light_pdf = state.light_pdf;
        this.rc_next_vertex_bounce_seed = state.rc_next_vertex_bounce_seed;
        this.reconnection_hit = state.reconnection_hit;
        this.path_flags = state.path_flags;
        this.reconnection_jacobian = state.reconnection_jacobian;
        this.reconnection_incident_radiance = state.reconnection_incident_radiance;
        this.reconnection_wi = state.reconnection_wi;
    }

    public static func create_from_ris_state(const state: PathRISState, init_random_seed: uint)
        ->PathReservoir
    {
        var r : PathReservoir;
        r.sample_mass = 1;
        r.weight = state.weight;
        r.integrand = state.integrand;
        r.light_pdf = state.light_pdf;
        r.rc_next_vertex_bounce_seed = state.rc_next_vertex_bounce_seed;
        r.init_random_seed = init_random_seed;
        r.reconnection_hit = state.reconnection_hit;
        r.path_flags = state.path_flags;
        r.reconnection_jacobian = state.reconnection_jacobian;
        r.reconnection_incident_radiance = state.reconnection_incident_radiance;
        r.reconnection_wi = state.reconnection_wi;
        return r;
    }

    [mutating]
    public func add(const state: PathRISState, init_random_seed: uint, u: float)->bool
    {
        var w : float = state.weight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;
        this.sample_mass += 1;
        this.weight += w;

        let sample_selected : bool = (u * this.weight < w);

        this.path_flags.flags = sample_selected ? state.path_flags.flags : this.path_flags.flags;
        this.rc_next_vertex_bounce_seed =
            sample_selected ? state.rc_next_vertex_bounce_seed : this.rc_next_vertex_bounce_seed;
        this.init_random_seed = sample_selected ? init_random_seed : this.init_random_seed;
        this.light_pdf = sample_selected ? state.light_pdf : this.light_pdf;

        if (sample_selected)
        {
            this.reconnection_hit = state.reconnection_hit;
        }
        this.integrand = sample_selected ? state.integrand : this.integrand;
        this.reconnection_jacobian =
            sample_selected ? state.reconnection_jacobian : this.reconnection_jacobian;
        this.reconnection_incident_radiance = sample_selected ? state.reconnection_incident_radiance
                                                              : this.reconnection_incident_radiance;
        this.reconnection_wi = sample_selected ? state.reconnection_wi : this.reconnection_wi;

        return sample_selected;
    }

    [mutating]
    public func merge(integrand: float3, in_jacobian: float, in_reservoir: PathReservoir, u: float,
                      mis_weight: float)
        ->bool
    {
        var w : float = PathRISState::to_scalar(integrand) * in_jacobian * in_reservoir.weight *
                        mis_weight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;

        this.sample_mass += in_reservoir.sample_mass;
        this.weight += w;

        let sample_selected : bool = (u * this.weight < w);

        this.path_flags.flags =
            sample_selected ? in_reservoir.path_flags.flags : this.path_flags.flags;
        this.rc_next_vertex_bounce_seed = sample_selected ? in_reservoir.rc_next_vertex_bounce_seed
                                                          : this.rc_next_vertex_bounce_seed;
        this.init_random_seed =
            sample_selected ? in_reservoir.init_random_seed : this.init_random_seed;
        this.light_pdf = sample_selected ? in_reservoir.light_pdf : this.light_pdf;
        this.reconnection_hit.data =
            sample_selected ? in_reservoir.reconnection_hit.data : this.reconnection_hit.data;
        this.integrand = sample_selected ? integrand : this.integrand;
        this.reconnection_jacobian =
            sample_selected ? in_reservoir.reconnection_jacobian : this.reconnection_jacobian;
        this.reconnection_incident_radiance = sample_selected
                                                  ? in_reservoir.reconnection_incident_radiance
                                                  : this.reconnection_incident_radiance;
        this.reconnection_wi =
            sample_selected ? in_reservoir.reconnection_wi : this.reconnection_wi;

        return sample_selected;
    }

    [mutating]
    public func merge_direct_overwrite(integrand: float3, in_reservoir: PathReservoir)->void
    {
        this.path_flags.flags = in_reservoir.path_flags.flags;
        this.rc_next_vertex_bounce_seed = in_reservoir.rc_next_vertex_bounce_seed;
        this.init_random_seed = in_reservoir.init_random_seed;
        this.light_pdf = in_reservoir.light_pdf;
        this.reconnection_hit.data = in_reservoir.reconnection_hit.data;
        this.integrand = integrand;
        this.reconnection_jacobian = in_reservoir.reconnection_jacobian;
        this.reconnection_incident_radiance = in_reservoir.reconnection_incident_radiance;
        this.reconnection_wi = in_reservoir.reconnection_wi;
    }

    [mutating]
    public func merge_without_writing(p_hat_with_jacobian: float, in_reservoir: PathReservoir,
                                      u: float, mis_weight: float)
        ->bool
    {
        var w : float = p_hat_with_jacobian * in_reservoir.weight * mis_weight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;
        this.sample_mass += in_reservoir.sample_mass;
        this.weight += w;
        let sample_selected : bool = (u * this.weight < w);
        return sample_selected;
    }

    [mutating]
    public func finalize_reservoir_weight()->void
    {
        let p_hat = PathRISState::to_scalar(integrand);
        this.weight = (p_hat == 0.f) ? 0.f : (this.weight / p_hat);
    }
}

/// 60B前缀重放结果；pathThroughput不含连接顶点重算，wo指离开rc前顶点。
public struct ReconnectionData
{
    public var rc_prev_hit : HitInfo;
    public var path_throughput : float3;
    public var rc_prev_wo : float3;
    public var rc_footprint_factor : float;
    /// 前缀实际到达该点的IOR与消光系数；仅凭该点材质和面朝向不能恢复。
    public var rc_prev_incident : float4;
};

/// 20B配对shift缓存；pathThroughput沿用NV命名，实际为目标域完整shift integrand。
public struct PrecomputedShiftData
{
    public var path_throughput : float3;
    public var jacobian : float;
    public var reconnection_jacobian : float;
};

/// 每像素有效邻居mask及sum sample_mass；默认只使用低3bits。
public struct NeighborValidMask
{
    public var data : uint;
    public var c_sum : float;
    public func is_valid(index: uint)->bool
    {
        return ((data >> index) & 1u) != 0;
    }
    [mutating]
    public func set_valid(index: uint, value: bool)->void
    {
        data = (data & ~(1u << index)) | (uint(value) << index);
    }
};
}
