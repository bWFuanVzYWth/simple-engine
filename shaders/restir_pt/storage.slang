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

/// 显式80/60/20B raw ABI，避免structured float3的后端布局或scalarBlockLayout feature隐含前提。
/// host按这些逻辑stride分配；index乘法与末端访问须在实际buffer容量内。无量化、打包损失或边界钳制。
module storage;
import restir_pt.types;

namespace restir_pt
{
public static const uint RESERVOIR_STRIDE = 80;
public static const uint RECONNECTION_STRIDE = 60;
public static const uint PRECOMPUTED_STRIDE = 20;

/// 五组连续uint4覆盖全部80B；拼接只解释位模式，不改变任何FP32精度。
PathReservoir unpack_reservoir(uint4 a, uint4 b, uint4 c, uint4 d, uint4 e)
{
    PathReservoir result;
    result.M = asfloat(a.x);
    result.weight = asfloat(a.y);
    result.integrand = asfloat(uint3(a.zw, b.x));
    result.pathFlags.flags = b.y;
    result.initRandomSeed = b.z;
    result.rcNextVertexBounceSeed = b.w;
    result.lightPdf = asfloat(c.x);
    result.reconnectionHit.data = uint4(c.yzw, d.x);
    result.reconnectionJacobian = asfloat(d.y);
    result.reconnectionIncidentRadiance = asfloat(uint3(d.zw, e.x));
    result.reconnectionWi = asfloat(e.yzw);
    return result;
}

/// 从ByteAddressBuffer读取一份80B状态；保留全部float位模式。
public PathReservoir load_reservoir(ByteAddressBuffer buffer, uint index)
{
    uint offset = index * RESERVOIR_STRIDE;
    return unpack_reservoir(buffer.Load4(offset), buffer.Load4(offset + 16u),
                            buffer.Load4(offset + 32u), buffer.Load4(offset + 48u),
                            buffer.Load4(offset + 64u));
}

/// 从RWByteAddressBuffer读取一份80B状态；保留全部float位模式。
public PathReservoir load_reservoir(RWByteAddressBuffer buffer, uint index)
{
    uint offset = index * RESERVOIR_STRIDE;
    return unpack_reservoir(buffer.Load4(offset), buffer.Load4(offset + 16u),
                            buffer.Load4(offset + 32u), buffer.Load4(offset + 48u),
                            buffer.Load4(offset + 64u));
}

/// 写入五组连续uint4；ABI仍是无填充的80B，不把热冷划分变成两份全屏分配。
public void store_reservoir(RWByteAddressBuffer buffer, uint index, PathReservoir value)
{
    uint offset = index * RESERVOIR_STRIDE;
    buffer.Store4(offset, uint4(asuint(value.M), asuint(value.weight), asuint(value.integrand.xy)));
    buffer.Store4(offset + 16u, uint4(asuint(value.integrand.z), value.pathFlags.flags,
                                      value.initRandomSeed, value.rcNextVertexBounceSeed));
    buffer.Store4(offset + 32u, uint4(asuint(value.lightPdf), value.reconnectionHit.data.xyz));
    buffer.Store4(offset + 48u,
                  uint4(value.reconnectionHit.data.w, asuint(value.reconnectionJacobian),
                        asuint(value.reconnectionIncidentRadiance.xy)));
    buffer.Store4(offset + 64u, uint4(asuint(value.reconnectionIncidentRadiance.z),
                                      asuint(value.reconnectionWi)));
}

/// MIS/CV只读取前20B；其余字段规范为零，最终赢家确定后才能恢复冷身份。
public PathReservoir load_merge_source(RWByteAddressBuffer buffer, uint index)
{
    uint offset = RESERVOIR_STRIDE * index;
    uint4 words = buffer.Load4(offset);
    PathReservoir result = {};
    result.M = asfloat(words.x);
    result.weight = asfloat(words.y);
    result.integrand = asfloat(uint3(words.zw, buffer.Load(offset + 16u)));
    return result;
}

/// prefix重放仅消费UCW、flags与初始种子；完整源在重放结束后重读。
public PathReservoir load_replay_source(RWByteAddressBuffer buffer, uint index)
{
    uint offset = RESERVOIR_STRIDE * index;
    PathReservoir source = {};
    source.weight = asfloat(buffer.Load(offset + 4u));
    uint2 identity = buffer.Load2(offset + 20u);
    source.pathFlags.flags = identity.x;
    source.initRandomSeed = identity.y;
    return source;
}

ReconnectionData unpack_reconnection(uint4 a, uint4 b, uint3 c, uint4 medium)
{
    ReconnectionData result;
    result.rcPrevHit.data = a;
    result.pathThroughput = asfloat(b.xyz);
    result.rcPrevWo = asfloat(uint3(b.w, c.xy));
    result.rcFootprintFactor = asfloat(c.z);
    result.rcPrevIncident = asfloat(medium);
    return result;
}

/// 从ByteAddressBuffer读取一份60B状态；介质紧接原有44B几何和throughput。
public ReconnectionData load_reconnection(ByteAddressBuffer buffer, uint index)
{
    uint offset = index * RECONNECTION_STRIDE;
    return unpack_reconnection(buffer.Load4(offset), buffer.Load4(offset + 16u),
                               buffer.Load3(offset + 32u), buffer.Load4(offset + 44u));
}

/// 从RWByteAddressBuffer读取一份60B状态；保留全部float位模式。
public ReconnectionData load_reconnection(RWByteAddressBuffer buffer, uint index)
{
    uint offset = index * RECONNECTION_STRIDE;
    return unpack_reconnection(buffer.Load4(offset), buffer.Load4(offset + 16u),
                               buffer.Load3(offset + 32u), buffer.Load4(offset + 44u));
}

/// 写入完整60B；不访问相邻记录，也不为16B事务增加填充。
public void store_reconnection(RWByteAddressBuffer buffer, uint index, ReconnectionData value)
{
    uint offset = index * RECONNECTION_STRIDE;
    buffer.Store4(offset, value.rcPrevHit.data);
    buffer.Store4(offset + 16u, uint4(asuint(value.pathThroughput), asuint(value.rcPrevWo.x)));
    buffer.Store3(offset + 32u, uint3(asuint(value.rcPrevWo.yz), asuint(value.rcFootprintFactor)));
    buffer.Store4(offset + 44u, asuint(value.rcPrevIncident));
}

PrecomputedShiftData unpack_precomputed(uint4 a, uint b)
{
    return { asfloat(a.xyz), asfloat(a.w), asfloat(b) };
}

/// 从ByteAddressBuffer读取一份20B状态；前16B一次读取。
public PrecomputedShiftData load_precomputed(ByteAddressBuffer buffer, uint index)
{
    uint offset = index * PRECOMPUTED_STRIDE;
    return unpack_precomputed(buffer.Load4(offset), buffer.Load(offset + 16u));
}

/// 从RWByteAddressBuffer读取一份20B状态；保留全部float位模式。
public PrecomputedShiftData load_precomputed(RWByteAddressBuffer buffer, uint index)
{
    uint offset = index * PRECOMPUTED_STRIDE;
    return unpack_precomputed(buffer.Load4(offset), buffer.Load(offset + 16u));
}

/// 写入一份20B状态；不把20B记录扩大到32B，不损失Jacobian精度。
public void store_precomputed(RWByteAddressBuffer buffer, uint index, PrecomputedShiftData value)
{
    uint offset = index * PRECOMPUTED_STRIDE;
    buffer.Store4(offset, uint4(asuint(value.pathThroughput), asuint(value.Jacobian)));
    buffer.Store(offset + 16u, asuint(value.reconnectionJacobian));
}

/// 颜色拥有独立的12B工作集，与相应reservoir使用同一padded Morton地址。
public float3 load_color(RWByteAddressBuffer buffer, uint index)
{
    return asfloat(buffer.Load3(12u * index));
}

/// 写入当前阶段独占的RGB元素；合法负颜色与全部FP32位模式保留。
public void store_color(RWByteAddressBuffer buffer, uint index, float3 color)
{
    buffer.Store3(12u * index, asuint(color));
}
}
