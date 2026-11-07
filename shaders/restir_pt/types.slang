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

/// Falcor 759aad03 默认 point ReSTIR PT 的未压缩中立ABI；scalar布局为80/44/20B。
/// identity的全0保留为无效，scene负责其余uint4编码、稳定性与重建，不保存宿主指针。
module types;

namespace restir_pt
{
/// NV PathTracerUtils::LightType的2-bit编码。
public static const uint LIGHT_ENV = 0;
public static const uint LIGHT_EMISSIVE = 1;
public static const uint LIGHT_ANALYTIC = 2;
public static const uint MAXIMUM_RC_LENGTH = 255;

/// 默认point Hybrid设置；distance阈值已按NV的c/100换算。
public struct ShiftSettings
{
    /// 0为纯reconnection，1为Hybrid random replay/reconnection。
    public uint shiftMapping = 1;
    public uint localStrategyType = 3;
    public float specularRoughnessThreshold = 0.2f;
    public float nearFieldDistanceThreshold = 0.0002f;
    public float specularRoughnessSigma = 0.0f;
    public float nearFieldDistanceSigma = 0.2f;
    public bool additionalShadowRayOffset = false;
};

/// 16B稳定交点身份；几何、材质、重心的解释归scene适配器。
public struct HitInfo
{
    /// 全0表示无效；有效身份须至少有一个非零分量。
    public uint4 data = 0;
    /// 是否有有效身份，不访问scene资源。
    public bool isValid()
    {
        return any(data != 0);
    }
    /// 与isValid等价的简短入口。
    public bool valid()
    {
        return isValid();
    }
};

/// NV原位布局：0..7为path length，8..15为rc length，16为NEE，17为primary direct，
/// 18..19为lightType，20..25为rc前delta/transmission/component/indexing，
/// 26..30为rc后delta/transmission/component，31为末顶点NEE无BSDF竞争；
/// 255是无可重连顶点的rc sentinel。
public struct ReSTIRPathFlags
{

    public static const uint kPathLengthMask = 0xFF;
    public static const uint kPathLengthShift = 0;
    public static const uint kRcVertexLengthMask = 0xFF;
    public static const uint kRcVertexLengthShift = 8;
    public static const uint kLastVertexNEEShift = 16;
    public static const uint kIsVeryDirectLightShift = 17;
    public static const uint kLightTypeMask = 0x3;
    public static const uint kLightTypeShift = 18;

    public static const uint kBouncePackedBeforeRcMask = 0x3f;
    public static const uint kBouncePackedBeforeRcShift = 20;
    public static const uint kIsDeltaBeforeRcShift = 20;
    public static const uint kIsTransmissionBeforeRcShift = 21;
    public static const uint kBSDFComponentBeforeRcShift = 22;
    public static const uint kSupportBSDFIndexingShift = 25;

    public static const uint kBouncePackedAfterRcMask = 0x1f;
    public static const uint kBouncePackedAfterRcShift = 26;
    public static const uint kIsDeltaAfterRcShift = 26;
    public static const uint kIsTransmissionAfterRcShift = 27;
    public static const uint kBSDFComponentAfterRcShift = 28;

    public static const uint kBSDFComponentMask = 0x7;
    public static const uint kTerminalNEEShift = 31;

    public uint flags = 0;

    [mutating]
    public void tryInsertBounceTypesBeforeRcVertex(uint data, bool doInsert)
    {
        flags &= doInsert ? ~(kBouncePackedBeforeRcMask << kBouncePackedBeforeRcShift) : 0xffffffff;
        flags |= doInsert ? (data << kBouncePackedBeforeRcShift) : 0;
    }

    [mutating]
    public void tryInsertBounceTypesAfterRcVertex(uint data, bool doInsert)
    {
        flags &= doInsert ? ~(kBouncePackedAfterRcMask << kBouncePackedAfterRcShift) : 0xffffffff;
        flags |= doInsert ? (data << kBouncePackedAfterRcShift) : 0;
    }

    [mutating]
    public void insertIsDeltaEvent(bool isDeltaEvent, bool beforeRcVertex)
    {
        const uint shift = beforeRcVertex ? kIsDeltaBeforeRcShift : kIsDeltaAfterRcShift;
        flags &= ~(1 << shift);
        flags |= uint(isDeltaEvent) << shift;
    }

    [mutating]
    public void insertIsTransmissionEvent(bool isTransmissionEvent, bool beforeRcVertex)
    {
        const uint shift =
            beforeRcVertex ? kIsTransmissionBeforeRcShift : kIsTransmissionAfterRcShift;
        flags &= ~(1 << shift);
        flags |= uint(isTransmissionEvent) << shift;
    }

    [mutating]
    public void insertBSDFComponentType(uint component, bool beforeRcVertex)
    {
        const uint shift =
            beforeRcVertex ? kBSDFComponentBeforeRcShift : kBSDFComponentAfterRcShift;
        flags &= ~(kBSDFComponentMask << shift);
        flags |= component << shift;
    }

    [mutating]
    public void insertSupportBSDFIndexingBeforeRcVertex(bool enabled)
    {
        flags &= ~(1 << kSupportBSDFIndexingShift);
        flags |= uint(enabled) << kSupportBSDFIndexingShift;
    }

    public bool decodeIsDeltaEvent(bool beforeRcVertex)
    {
        return ((flags >> (beforeRcVertex ? kIsDeltaBeforeRcShift : kIsDeltaAfterRcShift)) & 1) !=
               0u;
    }

    public bool decodeIsTransmissionEvent(bool beforeRcVertex)
    {
        return ((flags >>
                 (beforeRcVertex ? kIsTransmissionBeforeRcShift : kIsTransmissionAfterRcShift)) &
                1) != 0u;
    }

    public uint decodeBSDFComponent(bool beforeRcVertex)
    {
        return (flags >>
                (beforeRcVertex ? kBSDFComponentBeforeRcShift : kBSDFComponentAfterRcShift)) &
               kBSDFComponentMask;
    }

    public bool decodeSupportBSDFIndexingBeforeRcVertex()
    {
        return ((flags >> kSupportBSDFIndexingShift) & 1) != 0u;
    }

    [mutating]
    public void insertPathLength(int pathLength)
    {
        flags &= ~kPathLengthMask;
        flags |= pathLength & kPathLengthMask;
    }

    [mutating]
    public void insertRcVertexLength(int rcVertexLength)
    {
        flags &= ~(kRcVertexLengthMask << kRcVertexLengthShift);
        flags |= (rcVertexLength & kRcVertexLengthMask) << kRcVertexLengthShift;
    }

    [mutating]
    public void resetRcVertexLength()
    {
        flags |= kRcVertexLengthMask << kRcVertexLengthShift;
    }

    public int pathLength()
    {
        return flags & kPathLengthMask;
    }

    public int rcVertexLength()
    {
        return (flags >> kRcVertexLengthShift) & kRcVertexLengthMask;
    }

    [mutating]
    public void insertLastVertexNEE(bool isNEE)
    {
        flags &= ~(1 << kLastVertexNEEShift);
        flags |= (int(isNEE) & 1) << kLastVertexNEEShift;
    }

    public bool lastVertexNEE()
    {
        return ((flags >> kLastVertexNEEShift) & 1) != 0u;
    }

    /// 散射预算耗尽后的NEE没有BSDF采样竞争；随赢家保存，预算热切不能改写旧样本。
    [mutating]
    public void insertTerminalNEE(bool terminal)
    {
        flags = (flags & ~(1u << kTerminalNEEShift)) | (uint(terminal) << kTerminalNEEShift);
    }

    public bool terminalNEE()
    {
        return (flags >> kTerminalNEEShift) != 0u;
    }

    [mutating]
    public void insertIsVeryDirectLight(bool isVeryDirectLight)
    {
        flags &= ~(1 << kIsVeryDirectLightShift);
        flags |= (int(isVeryDirectLight) & 1) << kIsVeryDirectLightShift;
    }

    public bool isVeryDirectLight()
    {
        return ((flags >> kIsVeryDirectLightShift) & 1) != 0u;
    }

    [mutating]
    public void insertLightType(uint lightType)
    {
        flags &= ~(kLightTypeMask << kLightTypeShift);
        flags |= ((int(lightType) & kLightTypeMask) << kLightTypeShift);
    }

    public uint lightType()
    {
        return (flags >> kLightTypeShift) & kLightTypeMask;
    }
};

public struct LastVertexState
{
    public static const uint kIsDeltaShift = 0;
    public static const uint kIsTransmissionShift = 1;
    public static const uint kBSDFComponentShift = 2;
    public static const uint kBSDFComponentMask = 0x7;
    public static const uint kSupportBSDFIndexingShift = 5;
    public static const uint kIsRoughShift = 6;
    public static const uint kIsFarFromPrevShift = 7;

    public uint data = 0;

    [mutating]
    public void Init(bool isLastVertexDelta, bool isLastVertexTransmission, uint lastBSDFComponent,
                     bool lastVertexSupportBSDFIndexing, bool isLastVertexRough,
                     bool isCurrentVertexFarFromPrev)
    {
        data = 0;
        data |= (uint)isLastVertexDelta << kIsDeltaShift;
        data |= (uint)isLastVertexTransmission << kIsTransmissionShift;
        data |= (uint)lastBSDFComponent << kBSDFComponentShift;
        data |= (uint)lastVertexSupportBSDFIndexing << kSupportBSDFIndexingShift;
        data |= (uint)isLastVertexRough << kIsRoughShift;
        data |= (uint)isCurrentVertexFarFromPrev << kIsFarFromPrevShift;
    }

    public bool isLastVertexDelta()
    {
        return ((data >> kIsDeltaShift) & 1) != 0u;
    }
    public bool isLastVertexTransmission()
    {
        return ((data >> kIsTransmissionShift) & 1) != 0u;
    }
    public uint lastBSDFComponent()
    {
        return (data >> kBSDFComponentShift) & kBSDFComponentMask;
    }
    public bool lastVertexSupportBSDFIndexing()
    {
        return ((data >> kSupportBSDFIndexingShift) & 1) != 0u;
    }
    public bool isLastVertexRough()
    {
        return ((data >> kIsRoughShift) & 1) != 0u;
    }
    public bool isCurrentVertexFarFromPrev()
    {
        return ((data >> kIsFarFromPrevShift) & 1) != 0u;
    }
}

/// 只在当前追踪中使用的rc前缀状态；不是跨帧buffer ABI。
public struct PathReconnectionInfo
{
    public ReSTIRPathFlags pathFlags = {};
    public uint rcNextVertexBounceSeed = 0;
    public uint initRandomSeed = 0;
    public float3 rcThroughput = 0;
    public HitInfo reconnectionHit = {};
    public float reconnectionJacobian = 0;
    public float3 reconnectionWi = 0;
    public float footprintFactor = 0;
}

/// 路径内部候选RIS，weight为未归一化和；混合PSS/solid-angle integrand由scene形成。
public struct PathRISState
{
    public float weight = 0;
    public float3 integrand = 0;
    public ReSTIRPathFlags pathFlags = {};
    public float lightPdf = 0;
    public uint rcNextVertexBounceSeed = 0;
    public HitInfo reconnectionHit = {};
    public float reconnectionJacobian = 0;
    public float3 reconnectionIncidentRadiance = 0;
    public float3 reconnectionWi = 0;

    [mutating]
    public void init()
    {
        weight = 0.f;
    }

    public static float gaussrand(float mean, float stddev, float2 randomUV)
    {

        const float2 rand = randomUV;
        float Z = sqrt(-2.0 * log(rand.x)) * cos(2.0 * 3.14159265358979323846f * rand.y);
        float X = Z * stddev * mean + mean;
        return X;
    }

    /// 叶子已按发光、局部灯、环境顺序抽取胜者。此处仅写胜者的重连属性。
    [mutating]
    public void storeNEECandidate(PathReconnectionInfo rc, bool isRcVertex, uint depth,
                                  float3 contribution, float3 wi, float3 Le, float3 Lr,
                                  float sourcePdf, float pdf, uint lightType)
    {
        pathFlags = rc.pathFlags;
        pathFlags.insertPathLength(depth);
        if (isRcVertex)
            pathFlags.insertRcVertexLength(depth);
        pathFlags.insertLastVertexNEE(true);
        pathFlags.insertLightType(lightType);
        reconnectionIncidentRadiance = isRcVertex ? Le : rc.rcThroughput * Lr;
        reconnectionWi = isRcVertex ? wi : rc.reconnectionWi;
        reconnectionHit = rc.reconnectionHit;
        reconnectionJacobian = rc.reconnectionJacobian;
        rcNextVertexBounceSeed = rc.rcNextVertexBounceSeed;
        integrand = contribution * sourcePdf;
        lightPdf = pdf;
    }

    [mutating]
    public void storeBSDFHitCandidate(PathReconnectionInfo rc, uint depth, float3 contribution,
                                      float3 Le, float3 Lr, float sourcePdf, float pdf,
                                      uint lightType)
    {
        pathFlags = rc.pathFlags;
        pathFlags.insertPathLength(depth);
        pathFlags.insertLastVertexNEE(false);
        pathFlags.insertLightType(lightType);
        reconnectionIncidentRadiance =
            depth == rc.pathFlags.rcVertexLength() ? Le : rc.rcThroughput * Lr;
        reconnectionWi = rc.reconnectionWi;
        reconnectionHit = rc.reconnectionHit;
        reconnectionJacobian = rc.reconnectionJacobian;
        rcNextVertexBounceSeed = rc.rcNextVertexBounceSeed;
        integrand = contribution * sourcePdf;
        lightPdf = pdf;
    }

    [mutating]
    public bool addBSDFHitCandidate(float u, PathReconnectionInfo rc, uint depth,
                                    float3 contribution, float3 wi, float3 Le, float3 Lr,
                                    float sourcePdf, float pdf, uint lightType)
    {
        if (!addPSSSample(u, contribution))
            return false;
        storeBSDFHitCandidate(rc, depth, contribution, Le, Lr, sourcePdf, pdf, lightType);
        return true;
    }

    [mutating]
    public bool addPrimaryVisibleEmitterCandidate(float u, float3 pathContribution, float sourcePdf)
    {
        bool selected = addPSSSample(u, pathContribution);
        this.pathFlags.flags = selected ? 0 : this.pathFlags.flags;
        this.pathFlags.insertIsVeryDirectLight(selected ? true
                                                        : this.pathFlags.isVeryDirectLight());

        this.integrand = selected ? pathContribution * sourcePdf : this.integrand;
        return selected;
    }

    [mutating]
    public void markAsRcVertex(uint pathLength, HitInfo hit, LastVertexState lastVertexState,
                               float lightPdf, uint lightType, float3 Le, float3 wi,
                               float reconnectionJacobian)
    {
        this.pathFlags.insertRcVertexLength(pathLength);

        this.reconnectionHit = hit;
        this.pathFlags.insertIsDeltaEvent(lastVertexState.isLastVertexDelta(), true);
        this.pathFlags.insertIsTransmissionEvent(lastVertexState.isLastVertexTransmission(), true);
        this.pathFlags.insertBSDFComponentType(lastVertexState.lastBSDFComponent(), true);
        this.pathFlags.insertSupportBSDFIndexingBeforeRcVertex(
            lastVertexState.lastVertexSupportBSDFIndexing());
        this.lightPdf = lightPdf;

        this.reconnectionJacobian = reconnectionJacobian;

        this.pathFlags.insertLightType(lightType);
        this.reconnectionIncidentRadiance = Le;
        this.reconnectionWi = wi;
    }

    [mutating]
    public bool addPSSSample(float u, float3 pathContribution)
    {
        return selectCandidate(weight, u, pathContribution);
    }

    public static bool selectCandidate(inout float total, float u, float3 pathContribution)
    {
        float w = toScalar(pathContribution);

        w = isnan(w) ? 0.0f : w;
        total += w;

        return (u * total < w);
    }

    public static float toScalar(float3 color)
    {
        return dot(color, float3(0.299, 0.587, 0.114));
    }
}

/// 单像素canonical reservoir；weight构建时为和，finalize后为W。
/// M为候选/历史有效量，seed及rc后随机种子用于确定性重放。
public struct PathReservoir
{
    public float M;
    public float weight;
    public float3 integrand;
    public ReSTIRPathFlags pathFlags;
    public uint initRandomSeed;
    public uint rcNextVertexBounceSeed;
    public float lightPdf;
    public HitInfo reconnectionHit;
    public float reconnectionJacobian;
    public float3 reconnectionIncidentRadiance;
    public float3 reconnectionWi;

    [mutating]
    public void init()
    {
        this.M = 0;
        this.weight = 0;
        this.integrand = 0;
        this.pathFlags.flags = 0;
        this.initRandomSeed = 0;
        this.rcNextVertexBounceSeed = 0;
        this.lightPdf = 0;
        this.reconnectionHit.data = 0;
        this.reconnectionJacobian = 0;
        this.reconnectionIncidentRadiance = 0;
        this.reconnectionWi = 0;
    }

    [mutating]
    public void init(const PathRISState state)
    {
        this.M = 1;
        this.weight = state.weight;
        this.integrand = state.integrand;
        this.lightPdf = state.lightPdf;
        this.rcNextVertexBounceSeed = state.rcNextVertexBounceSeed;
        this.reconnectionHit = state.reconnectionHit;
        this.pathFlags = state.pathFlags;
        this.reconnectionJacobian = state.reconnectionJacobian;
        this.reconnectionIncidentRadiance = state.reconnectionIncidentRadiance;
        this.reconnectionWi = state.reconnectionWi;
    }

    public static PathReservoir createFromRisState(const PathRISState state, uint initRandomSeed)
    {
        PathReservoir r;
        r.M = 1;
        r.weight = state.weight;
        r.integrand = state.integrand;
        r.lightPdf = state.lightPdf;
        r.rcNextVertexBounceSeed = state.rcNextVertexBounceSeed;
        r.initRandomSeed = initRandomSeed;
        r.reconnectionHit = state.reconnectionHit;
        r.pathFlags = state.pathFlags;
        r.reconnectionJacobian = state.reconnectionJacobian;
        r.reconnectionIncidentRadiance = state.reconnectionIncidentRadiance;
        r.reconnectionWi = state.reconnectionWi;
        return r;
    }

    [mutating]
    public bool add(const PathRISState state, uint initRandomSeed, float u)
    {
        float w = state.weight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;
        this.M += 1;
        this.weight += w;

        bool sampleSelected = (u * this.weight < w);

        this.pathFlags.flags = sampleSelected ? state.pathFlags.flags : this.pathFlags.flags;
        this.rcNextVertexBounceSeed =
            sampleSelected ? state.rcNextVertexBounceSeed : this.rcNextVertexBounceSeed;
        this.initRandomSeed = sampleSelected ? initRandomSeed : this.initRandomSeed;
        this.lightPdf = sampleSelected ? state.lightPdf : this.lightPdf;

        if (sampleSelected)
        {
            this.reconnectionHit = state.reconnectionHit;
        }
        this.integrand = sampleSelected ? state.integrand : this.integrand;
        this.reconnectionJacobian =
            sampleSelected ? state.reconnectionJacobian : this.reconnectionJacobian;
        this.reconnectionIncidentRadiance =
            sampleSelected ? state.reconnectionIncidentRadiance : this.reconnectionIncidentRadiance;
        this.reconnectionWi = sampleSelected ? state.reconnectionWi : this.reconnectionWi;

        return sampleSelected;
    }

    [mutating]
    public bool merge(float3 integrand, float in_Jacobian, PathReservoir inReservoir, float u,
                      float misWeight)
    {
        float w = PathRISState::toScalar(integrand) * in_Jacobian * inReservoir.weight * misWeight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;

        this.M += inReservoir.M;
        this.weight += w;

        bool sampleSelected = (u * this.weight < w);

        this.pathFlags.flags = sampleSelected ? inReservoir.pathFlags.flags : this.pathFlags.flags;
        this.rcNextVertexBounceSeed =
            sampleSelected ? inReservoir.rcNextVertexBounceSeed : this.rcNextVertexBounceSeed;
        this.initRandomSeed = sampleSelected ? inReservoir.initRandomSeed : this.initRandomSeed;
        this.lightPdf = sampleSelected ? inReservoir.lightPdf : this.lightPdf;
        this.reconnectionHit.data =
            sampleSelected ? inReservoir.reconnectionHit.data : this.reconnectionHit.data;
        this.integrand = sampleSelected ? integrand : this.integrand;
        this.reconnectionJacobian =
            sampleSelected ? inReservoir.reconnectionJacobian : this.reconnectionJacobian;
        this.reconnectionIncidentRadiance = sampleSelected
                                                ? inReservoir.reconnectionIncidentRadiance
                                                : this.reconnectionIncidentRadiance;
        this.reconnectionWi = sampleSelected ? inReservoir.reconnectionWi : this.reconnectionWi;

        return sampleSelected;
    }

    [mutating]
    public void mergeDirectOverwrite(float3 integrand, PathReservoir inReservoir)
    {
        this.pathFlags.flags = inReservoir.pathFlags.flags;
        this.rcNextVertexBounceSeed = inReservoir.rcNextVertexBounceSeed;
        this.initRandomSeed = inReservoir.initRandomSeed;
        this.lightPdf = inReservoir.lightPdf;
        this.reconnectionHit.data = inReservoir.reconnectionHit.data;
        this.integrand = integrand;
        this.reconnectionJacobian = inReservoir.reconnectionJacobian;
        this.reconnectionIncidentRadiance = inReservoir.reconnectionIncidentRadiance;
        this.reconnectionWi = inReservoir.reconnectionWi;
    }

    [mutating]
    public bool mergeWithoutWriting(float pHatWithJacobian, PathReservoir inReservoir, float u,
                                    float misWeight)
    {
        float w = pHatWithJacobian * inReservoir.weight * misWeight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;
        this.M += inReservoir.M;
        this.weight += w;
        bool sampleSelected = (u * this.weight < w);
        return sampleSelected;
    }

    [mutating]
    public void finalizeReservoirWeight()
    {
        float p_hat = PathRISState::toScalar(integrand);
        this.weight = (p_hat == 0.f) ? 0.f : (this.weight / p_hat);
    }
}

/// 44B前缀重放结果；pathThroughput不含连接顶点重算，wo指离开rc前顶点。
public struct ReconnectionData
{
    public HitInfo rcPrevHit;
    public float3 pathThroughput;
    public float3 rcPrevWo;
    public float rcFootprintFactor;
};

/// 20B配对shift缓存；pathThroughput沿用NV命名，实际为目标域完整shift integrand。
public struct PrecomputedShiftData
{
    public float3 pathThroughput;
    public float Jacobian;
    public float reconnectionJacobian;
};

/// 每像素有效邻居mask及sum M；默认只使用低3bits。
public struct NeighborValidMask
{
    public uint data;
    public float cSum;
    public bool isValid(uint index)
    {
        return ((data >> index) & 1u) != 0;
    }
    [mutating]
    public void setValid(uint index, bool value)
    {
        data = (data & ~(1u << index)) | (uint(value) << index);
    }
};
}
