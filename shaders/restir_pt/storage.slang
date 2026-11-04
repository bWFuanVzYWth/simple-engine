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

/// 显式80/44/20B raw ABI，避免structured float3的后端布局或scalarBlockLayout feature隐含前提。
/// host按这些逻辑stride分配；index乘法与末端访问须在实际buffer容量内。无量化、打包损失或边界钳制。
module storage;
import restir_pt.types;

namespace restir_pt
{
public static const uint RESERVOIR_STRIDE = 80;
public static const uint RECONNECTION_STRIDE = 44;
public static const uint PRECOMPUTED_STRIDE = 20;

/// 从ByteAddressBuffer读取一份80B状态；保留全部float位模式。
public PathReservoir load_reservoir(ByteAddressBuffer buffer, uint index)
{
    uint offset = index * 80u;
    PathReservoir result;
    result.M = asfloat(buffer.Load(offset + 0u));
    result.weight = asfloat(buffer.Load(offset + 4u));
    result.integrand = asfloat(buffer.Load3(offset + 8u));
    result.pathFlags.flags = buffer.Load(offset + 20u);
    result.initRandomSeed = buffer.Load(offset + 24u);
    result.rcNextVertexBounceSeed = buffer.Load(offset + 28u);
    result.lightPdf = asfloat(buffer.Load(offset + 32u));
    result.reconnectionHit.data = buffer.Load4(offset + 36u);
    result.reconnectionJacobian = asfloat(buffer.Load(offset + 52u));
    result.reconnectionIncidentRadiance = asfloat(buffer.Load3(offset + 56u));
    result.reconnectionWi = asfloat(buffer.Load3(offset + 68u));
    return result;
}

/// 从RWByteAddressBuffer读取一份80B状态；保留全部float位模式。
public PathReservoir load_reservoir(RWByteAddressBuffer buffer, uint index)
{
    uint offset = index * 80u;
    PathReservoir result;
    result.M = asfloat(buffer.Load(offset + 0u));
    result.weight = asfloat(buffer.Load(offset + 4u));
    result.integrand = asfloat(buffer.Load3(offset + 8u));
    result.pathFlags.flags = buffer.Load(offset + 20u);
    result.initRandomSeed = buffer.Load(offset + 24u);
    result.rcNextVertexBounceSeed = buffer.Load(offset + 28u);
    result.lightPdf = asfloat(buffer.Load(offset + 32u));
    result.reconnectionHit.data = buffer.Load4(offset + 36u);
    result.reconnectionJacobian = asfloat(buffer.Load(offset + 52u));
    result.reconnectionIncidentRadiance = asfloat(buffer.Load3(offset + 56u));
    result.reconnectionWi = asfloat(buffer.Load3(offset + 68u));
    return result;
}

/// 写入一份80B状态；与load_reservoir共用逐字段固定偏移。
public void store_reservoir(RWByteAddressBuffer buffer, uint index, PathReservoir value)
{
    uint offset = index * 80u;
    buffer.Store(offset + 0u, asuint(value.M));
    buffer.Store(offset + 4u, asuint(value.weight));
    buffer.Store3(offset + 8u, asuint(value.integrand));
    buffer.Store(offset + 20u, value.pathFlags.flags);
    buffer.Store(offset + 24u, value.initRandomSeed);
    buffer.Store(offset + 28u, value.rcNextVertexBounceSeed);
    buffer.Store(offset + 32u, asuint(value.lightPdf));
    buffer.Store4(offset + 36u, value.reconnectionHit.data);
    buffer.Store(offset + 52u, asuint(value.reconnectionJacobian));
    buffer.Store3(offset + 56u, asuint(value.reconnectionIncidentRadiance));
    buffer.Store3(offset + 68u, asuint(value.reconnectionWi));
}

/// 从ByteAddressBuffer读取一份44B状态；保留全部float位模式。
public ReconnectionData load_reconnection(ByteAddressBuffer buffer, uint index)
{
    uint offset = index * 44u;
    ReconnectionData result;
    result.rcPrevHit.data = buffer.Load4(offset + 0u);
    result.pathThroughput = asfloat(buffer.Load3(offset + 16u));
    result.rcPrevWo = asfloat(buffer.Load3(offset + 28u));
    result.rcFootprintFactor = asfloat(buffer.Load(offset + 40u));
    return result;
}

/// 从RWByteAddressBuffer读取一份44B状态；保留全部float位模式。
public ReconnectionData load_reconnection(RWByteAddressBuffer buffer, uint index)
{
    uint offset = index * 44u;
    ReconnectionData result;
    result.rcPrevHit.data = buffer.Load4(offset + 0u);
    result.pathThroughput = asfloat(buffer.Load3(offset + 16u));
    result.rcPrevWo = asfloat(buffer.Load3(offset + 28u));
    result.rcFootprintFactor = asfloat(buffer.Load(offset + 40u));
    return result;
}

/// 写入一份44B状态；与load_reconnection共用逐字段固定偏移。
public void store_reconnection(RWByteAddressBuffer buffer, uint index, ReconnectionData value)
{
    uint offset = index * 44u;
    buffer.Store4(offset + 0u, value.rcPrevHit.data);
    buffer.Store3(offset + 16u, asuint(value.pathThroughput));
    buffer.Store3(offset + 28u, asuint(value.rcPrevWo));
    buffer.Store(offset + 40u, asuint(value.rcFootprintFactor));
}

/// 从ByteAddressBuffer读取一份20B状态；保留全部float位模式。
public PrecomputedShiftData load_precomputed(ByteAddressBuffer buffer, uint index)
{
    uint offset = index * 20u;
    PrecomputedShiftData result;
    result.pathThroughput = asfloat(buffer.Load3(offset + 0u));
    result.Jacobian = asfloat(buffer.Load(offset + 12u));
    result.reconnectionJacobian = asfloat(buffer.Load(offset + 16u));
    return result;
}

/// 从RWByteAddressBuffer读取一份20B状态；保留全部float位模式。
public PrecomputedShiftData load_precomputed(RWByteAddressBuffer buffer, uint index)
{
    uint offset = index * 20u;
    PrecomputedShiftData result;
    result.pathThroughput = asfloat(buffer.Load3(offset + 0u));
    result.Jacobian = asfloat(buffer.Load(offset + 12u));
    result.reconnectionJacobian = asfloat(buffer.Load(offset + 16u));
    return result;
}

/// 写入一份20B状态；与load_precomputed共用逐字段固定偏移。
public void store_precomputed(RWByteAddressBuffer buffer, uint index, PrecomputedShiftData value)
{
    uint offset = index * 20u;
    buffer.Store3(offset + 0u, asuint(value.pathThroughput));
    buffer.Store(offset + 12u, asuint(value.Jacobian));
    buffer.Store(offset + 16u, asuint(value.reconnectionJacobian));
}

}
