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
    public var shiftMapping : uint = 1;
    public var localStrategyType : uint = 3;
    public var specularRoughnessThreshold = 0.2f;
    public var nearFieldDistanceThreshold = 0.0002f;
    public var specularRoughnessSigma = 0.0f;
    public var nearFieldDistanceSigma = 0.2f;
    public var additionalShadowRayOffset = false;
};

/// 16B稳定交点身份；几何、材质、重心的解释归scene适配器。
public struct HitInfo
{
    /// 全0表示无效；有效身份须至少有一个非零分量。
    public var data : uint4 = 0;
    /// 是否有有效身份，不访问scene资源。
    public func isValid()->bool
    {
        return any(data != 0);
    }
    /// 与isValid等价的简短入口。
    public func valid()->bool
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
    public static const let kPathLengthMask : uint = 0xFF;
    public static const let kPathLengthShift : uint = 0;
    public static const let kRcVertexLengthMask : uint = 0xFF;
    public static const let kRcVertexLengthShift : uint = 8;
    public static const let kLastVertexNEEShift : uint = 16;
    public static const let kIsVeryDirectLightShift : uint = 17;
    public static const let kLightTypeMask : uint = 0x3;
    public static const let kLightTypeShift : uint = 18;

    public static const let kBouncePackedBeforeRcMask : uint = 0x3f;
    public static const let kBouncePackedBeforeRcShift : uint = 20;
    public static const let kIsDeltaBeforeRcShift : uint = 20;
    public static const let kIsTransmissionBeforeRcShift : uint = 21;
    public static const let kBSDFComponentBeforeRcShift : uint = 22;
    public static const let kSupportBSDFIndexingShift : uint = 25;

    public static const let kBouncePackedAfterRcMask : uint = 0x1f;
    public static const let kBouncePackedAfterRcShift : uint = 26;
    public static const let kIsDeltaAfterRcShift : uint = 26;
    public static const let kIsTransmissionAfterRcShift : uint = 27;
    public static const let kBSDFComponentAfterRcShift : uint = 28;

    public static const let kBSDFComponentMask : uint = 0x7;
    public static const let kTerminalNEEShift : uint = 31;

    public var flags : uint = 0;

    [mutating]
    public func tryInsertBounceTypesBeforeRcVertex(data: uint, doInsert: bool)->void
    {
        flags &= doInsert ? ~(kBouncePackedBeforeRcMask << kBouncePackedBeforeRcShift) : 0xffffffff;
        flags |= doInsert ? (data << kBouncePackedBeforeRcShift) : 0;
    }

    [mutating]
    public func tryInsertBounceTypesAfterRcVertex(data: uint, doInsert: bool)->void
    {
        flags &= doInsert ? ~(kBouncePackedAfterRcMask << kBouncePackedAfterRcShift) : 0xffffffff;
        flags |= doInsert ? (data << kBouncePackedAfterRcShift) : 0;
    }

    [mutating]
    public func insertIsDeltaEvent(isDeltaEvent: bool, beforeRcVertex: bool)->void
    {
        let shift : uint = beforeRcVertex ? kIsDeltaBeforeRcShift : kIsDeltaAfterRcShift;
        flags &= ~(1 << shift);
        flags |= uint(isDeltaEvent) << shift;
    }

    [mutating]
    public func insertIsTransmissionEvent(isTransmissionEvent: bool, beforeRcVertex: bool)->void
    {
        let shift : uint =
                        beforeRcVertex ? kIsTransmissionBeforeRcShift : kIsTransmissionAfterRcShift;
        flags &= ~(1 << shift);
        flags |= uint(isTransmissionEvent) << shift;
    }

    [mutating]
    public func insertBSDFComponentType(component: uint, beforeRcVertex: bool)->void
    {
        let shift : uint =
                        beforeRcVertex ? kBSDFComponentBeforeRcShift : kBSDFComponentAfterRcShift;
        flags &= ~(kBSDFComponentMask << shift);
        flags |= component << shift;
    }

    [mutating]
    public func insertSupportBSDFIndexingBeforeRcVertex(enabled: bool)->void
    {
        flags &= ~(1 << kSupportBSDFIndexingShift);
        flags |= uint(enabled) << kSupportBSDFIndexingShift;
    }

    public func decodeIsDeltaEvent(beforeRcVertex: bool)->bool
    {
        return ((flags >> (beforeRcVertex ? kIsDeltaBeforeRcShift : kIsDeltaAfterRcShift)) & 1) !=
               0u;
    }

    public func decodeIsTransmissionEvent(beforeRcVertex: bool)->bool
    {
        return ((flags >>
                 (beforeRcVertex ? kIsTransmissionBeforeRcShift : kIsTransmissionAfterRcShift)) &
                1) != 0u;
    }

    public func decodeBSDFComponent(beforeRcVertex: bool)->uint
    {
        return (flags >>
                (beforeRcVertex ? kBSDFComponentBeforeRcShift : kBSDFComponentAfterRcShift)) &
               kBSDFComponentMask;
    }

    public func decodeSupportBSDFIndexingBeforeRcVertex()->bool
    {
        return ((flags >> kSupportBSDFIndexingShift) & 1) != 0u;
    }

    [mutating]
    public func insertPathLength(pathLength: int)->void
    {
        flags &= ~kPathLengthMask;
        flags |= pathLength & kPathLengthMask;
    }

    [mutating]
    public func insertRcVertexLength(rcVertexLength: int)->void
    {
        flags &= ~(kRcVertexLengthMask << kRcVertexLengthShift);
        flags |= (rcVertexLength & kRcVertexLengthMask) << kRcVertexLengthShift;
    }

    [mutating]
    public func resetRcVertexLength()->void
    {
        flags |= kRcVertexLengthMask << kRcVertexLengthShift;
    }

    public func pathLength()->int
    {
        return flags & kPathLengthMask;
    }

    public func rcVertexLength()->int
    {
        return (flags >> kRcVertexLengthShift) & kRcVertexLengthMask;
    }

    [mutating]
    public func insertLastVertexNEE(isNEE: bool)->void
    {
        flags &= ~(1 << kLastVertexNEEShift);
        flags |= (int(isNEE) & 1) << kLastVertexNEEShift;
    }

    public func lastVertexNEE()->bool
    {
        return ((flags >> kLastVertexNEEShift) & 1) != 0u;
    }

    /// 散射预算耗尽后的NEE没有BSDF采样竞争；随赢家保存，预算热切不能改写旧样本。
    [mutating]
    public func insertTerminalNEE(terminal: bool)->void
    {
        flags = (flags & ~(1u << kTerminalNEEShift)) | (uint(terminal) << kTerminalNEEShift);
    }

    public func terminalNEE()->bool
    {
        return (flags >> kTerminalNEEShift) != 0u;
    }

    [mutating]
    public func insertIsVeryDirectLight(isVeryDirectLight: bool)->void
    {
        flags &= ~(1 << kIsVeryDirectLightShift);
        flags |= (int(isVeryDirectLight) & 1) << kIsVeryDirectLightShift;
    }

    public func isVeryDirectLight()->bool
    {
        return ((flags >> kIsVeryDirectLightShift) & 1) != 0u;
    }

    [mutating]
    public func insertLightType(lightType: uint)->void
    {
        flags &= ~(kLightTypeMask << kLightTypeShift);
        flags |= ((int(lightType) & kLightTypeMask) << kLightTypeShift);
    }

    public func lightType()->uint
    {
        return (flags >> kLightTypeShift) & kLightTypeMask;
    }
};

public struct LastVertexState
{
    public static const let kIsDeltaShift : uint = 0;
    public static const let kIsTransmissionShift : uint = 1;
    public static const let kBSDFComponentShift : uint = 2;
    public static const let kBSDFComponentMask : uint = 0x7;
    public static const let kSupportBSDFIndexingShift : uint = 5;
    public static const let kIsRoughShift : uint = 6;
    public static const let kIsFarFromPrevShift : uint = 7;

    public var data : uint = 0;

    [mutating]
    public func Init(isLastVertexDelta: bool, isLastVertexTransmission: bool,
                     lastBSDFComponent: uint, lastVertexSupportBSDFIndexing: bool,
                     isLastVertexRough: bool, isCurrentVertexFarFromPrev: bool)
        ->void
    {
        data = 0;
        data |= (uint)isLastVertexDelta << kIsDeltaShift;
        data |= (uint)isLastVertexTransmission << kIsTransmissionShift;
        data |= (uint)lastBSDFComponent << kBSDFComponentShift;
        data |= (uint)lastVertexSupportBSDFIndexing << kSupportBSDFIndexingShift;
        data |= (uint)isLastVertexRough << kIsRoughShift;
        data |= (uint)isCurrentVertexFarFromPrev << kIsFarFromPrevShift;
    }

    public func isLastVertexDelta()->bool
    {
        return ((data >> kIsDeltaShift) & 1) != 0u;
    }
    public func isLastVertexTransmission()->bool
    {
        return ((data >> kIsTransmissionShift) & 1) != 0u;
    }
    public func lastBSDFComponent()->uint
    {
        return (data >> kBSDFComponentShift) & kBSDFComponentMask;
    }
    public func lastVertexSupportBSDFIndexing()->bool
    {
        return ((data >> kSupportBSDFIndexingShift) & 1) != 0u;
    }
    public func isLastVertexRough()->bool
    {
        return ((data >> kIsRoughShift) & 1) != 0u;
    }
    public func isCurrentVertexFarFromPrev()->bool
    {
        return ((data >> kIsFarFromPrevShift) & 1) != 0u;
    }
}

/// 只在当前追踪中使用的rc前缀状态；不是跨帧buffer ABI。
public struct PathReconnectionInfo
{
    public var pathFlags : ReSTIRPathFlags = {};
    public var rcNextVertexBounceSeed : uint = 0;
    public var initRandomSeed : uint = 0;
    public var rcThroughput : float3 = 0;
    public var reconnectionHit : HitInfo = {};
    public var reconnectionJacobian : float = 0;
    public var reconnectionWi : float3 = 0;
    public var footprintFactor : float = 0;
}

/// 路径内部候选RIS，weight为未归一化和；混合PSS/solid-angle integrand由scene形成。
public struct PathRISState
{
    public var weight : float = 0;
    public var integrand : float3 = 0;
    public var pathFlags : ReSTIRPathFlags = {};
    public var lightPdf : float = 0;
    public var rcNextVertexBounceSeed : uint = 0;
    public var reconnectionHit : HitInfo = {};
    public var reconnectionJacobian : float = 0;
    public var reconnectionIncidentRadiance : float3 = 0;
    public var reconnectionWi : float3 = 0;

    [mutating]
    public func init()->void
    {
        weight = 0.f;
    }

    public static func gaussrand(mean: float, stddev: float, randomUV: float2)->float
    {
        let rand : float2 = randomUV;
        let Z : float = sqrt(-2.0 * log(rand.x)) * cos(2.0 * 3.14159265358979323846f * rand.y);
        let X : float = Z * stddev * mean + mean;
        return X;
    }

    /// 叶子已按发光、局部灯、环境顺序抽取胜者。此处仅写胜者的重连属性。
    [mutating]
    public func storeNEECandidate(rc: PathReconnectionInfo, isRcVertex: bool, depth: uint,
                                  contribution: float3, wi: float3, Le: float3, Lr: float3,
                                  sourcePdf: float, pdf: float, lightType: uint)
        ->void
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
    public func storeBSDFHitCandidate(rc: PathReconnectionInfo, depth: uint, contribution: float3,
                                      Le: float3, Lr: float3, sourcePdf: float, pdf: float,
                                      lightType: uint)
        ->void
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
    public func addBSDFHitCandidate(u: float, rc: PathReconnectionInfo, depth: uint,
                                    contribution: float3, wi: float3, Le: float3, Lr: float3,
                                    sourcePdf: float, pdf: float, lightType: uint)
        ->bool
    {
        if (!addPSSSample(u, contribution))
            return false;
        storeBSDFHitCandidate(rc, depth, contribution, Le, Lr, sourcePdf, pdf, lightType);
        return true;
    }

    [mutating]
    public func addPrimaryVisibleEmitterCandidate(u: float, pathContribution: float3,
                                                  sourcePdf: float)
        ->bool
    {
        let selected : bool = addPSSSample(u, pathContribution);
        this.pathFlags.flags = selected ? 0 : this.pathFlags.flags;
        this.pathFlags.insertIsVeryDirectLight(selected ? true
                                                        : this.pathFlags.isVeryDirectLight());

        this.integrand = selected ? pathContribution * sourcePdf : this.integrand;
        return selected;
    }

    [mutating]
    public func markAsRcVertex(pathLength: uint, hit: HitInfo, lastVertexState: LastVertexState,
                               lightPdf: float, lightType: uint, Le: float3, wi: float3,
                               reconnectionJacobian: float)
        ->void
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
    public func addPSSSample(u: float, pathContribution: float3)->bool
    {
        return selectCandidate(weight, u, pathContribution);
    }

    public static func selectCandidate(inout total: float, u: float, pathContribution: float3)->bool
    {
        var w : float = toScalar(pathContribution);

        w = isnan(w) ? 0.0f : w;
        total += w;

        return (u * total < w);
    }

    public static func toScalar(color: float3)->float
    {
        return dot(color, float3(0.299, 0.587, 0.114));
    }
}

/// 单像素canonical reservoir；weight构建时为和，finalize后为W。
/// M为候选/历史有效量，seed及rc后随机种子用于确定性重放。
public struct PathReservoir
{
    public var M : float;
    public var weight : float;
    public var integrand : float3;
    public var pathFlags : ReSTIRPathFlags;
    public var initRandomSeed : uint;
    public var rcNextVertexBounceSeed : uint;
    public var lightPdf : float;
    public var reconnectionHit : HitInfo;
    public var reconnectionJacobian : float;
    public var reconnectionIncidentRadiance : float3;
    public var reconnectionWi : float3;

    [mutating]
    public func init()->void
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
    public func init(const state: PathRISState)->void
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

    public static func createFromRisState(const state: PathRISState, initRandomSeed: uint)
        ->PathReservoir
    {
        var r : PathReservoir;
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
    public func add(const state: PathRISState, initRandomSeed: uint, u: float)->bool
    {
        var w : float = state.weight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;
        this.M += 1;
        this.weight += w;

        let sampleSelected : bool = (u * this.weight < w);

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
    public func merge(integrand: float3, in_Jacobian: float, inReservoir: PathReservoir, u: float,
                      misWeight: float)
        ->bool
    {
        var w : float = PathRISState::toScalar(integrand) * in_Jacobian * inReservoir.weight *
                        misWeight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;

        this.M += inReservoir.M;
        this.weight += w;

        let sampleSelected : bool = (u * this.weight < w);

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
    public func mergeDirectOverwrite(integrand: float3, inReservoir: PathReservoir)->void
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
    public func mergeWithoutWriting(pHatWithJacobian: float, inReservoir: PathReservoir, u: float,
                                    misWeight: float)
        ->bool
    {
        var w : float = pHatWithJacobian * inReservoir.weight * misWeight;
        w = (isnan(w) || isinf(w)) ? 0.0f : w;
        this.M += inReservoir.M;
        this.weight += w;
        let sampleSelected : bool = (u * this.weight < w);
        return sampleSelected;
    }

    [mutating]
    public func finalizeReservoirWeight()->void
    {
        let p_hat : float = PathRISState::toScalar(integrand);
        this.weight = (p_hat == 0.f) ? 0.f : (this.weight / p_hat);
    }
}

/// 60B前缀重放结果；pathThroughput不含连接顶点重算，wo指离开rc前顶点。
public struct ReconnectionData
{
    public var rcPrevHit : HitInfo;
    public var pathThroughput : float3;
    public var rcPrevWo : float3;
    public var rcFootprintFactor : float;
    /// 前缀实际到达该点的IOR与消光系数；仅凭该点材质和面朝向不能恢复。
    public var rcPrevIncident : float4;
};

/// 20B配对shift缓存；pathThroughput沿用NV命名，实际为目标域完整shift integrand。
public struct PrecomputedShiftData
{
    public var pathThroughput : float3;
    public var Jacobian : float;
    public var reconnectionJacobian : float;
};

/// 每像素有效邻居mask及sum M；默认只使用低3bits。
public struct NeighborValidMask
{
    public var data : uint;
    public var cSum : float;
    public func isValid(index: uint)->bool
    {
        return ((data >> index) & 1u) != 0;
    }
    [mutating]
    public func setValid(index: uint, value: bool)->void
    {
        data = (data & ~(1u << index)) | (uint(value) << index);
    }
};
}
