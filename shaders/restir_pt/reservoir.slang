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

/// NV默认point reservoir的纯RIS操作。u须来自独立的[0,1)均匀样本；不消费隐藏随机数。
/// 初始candidate采用mixed PSS/solid-angle integrand，weight为PSS路径贡献的标量RIS和。
module reservoir;
import restir_pt.types;

namespace restir_pt
{
/// NV默认标量target：BT.601 luminance；所有相关MIS target必须使用同一映射。
public float target(float3 integrand)
{
    return PathRISState::toScalar(integrand);
}

/// 清空整份80B状态；空reservoir的M、weight和identity均为0。
public PathReservoir empty_reservoir()
{
    PathReservoir result = {};
    result.init();
    return result;
}

/// 第一个已完成路径候选；seed必须是阈值/散射随机流消费前的可重放状态。
public PathReservoir create_initial_reservoir(PathRISState sample, uint seed)
{
    return PathReservoir::createFromRisState(sample, seed);
}

/// 加入后续初始路径；每个完成路径使M加1，返回其是否取代当前选中项。
public bool stream_ris(inout PathReservoir result, PathRISState sample, uint seed, float u)
{
    return result.add(sample, seed, u);
}

/// 保留source完整重放身份，以目标域integrand与Jacobian合并一个已finalize的source。
/// 调用方须先把source.reconnectionJacobian更新为本次连接的目标几何因子。
public bool merge_reservoir(inout PathReservoir result, PathReservoir source, float3 integrand,
                            float Jacobian, float misWeight, float u)
{
    return result.merge(integrand, Jacobian, source, u, misWeight);
}

/// 初始SPP结束：w_sum/(M*p_hat)，随后M设为1；与NV populateReservoir一致。
public void finalize_initial(inout PathReservoir result)
{
    float pHat = target(result.integrand);
    result.weight = pHat == 0.f ? 0.f : result.weight / (result.M * pHat);
    result.M = 1.f;
}

/// GRIS轮结束：w_sum/p_hat；不再除M，MIS已包含各source的有效量。
/// 无效/负的最终W清为0，保持NV重采样pass的最终数值处理。
public void finalize_reuse(inout PathReservoir result)
{
    result.finalizeReservoirWeight();
    result.weight = isnan(result.weight) || isinf(result.weight) ? 0.f : max(0.f, result.weight);
}
}
