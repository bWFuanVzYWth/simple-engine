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

import restir_pt.types;
import restir_pt.rng;
import restir_pt.scene;
import geometry.ray;

namespace restir_pt
{
/// 当前 bounce 的随机输入；light 的后两维是 NV 非 LightTileRIS 路径的固定 padding。
public struct ReplayRandom
{
    public float uBSDFRIS;
    public float uNEERIS;
    public uint neeSeed;
    public float lightSelector;
    public float4 light;
    public float roulette;
    /// xy 为方向，z 为 lobe/Fresnel 选择；RNG 中仍先消费选择维。
    public float3 bsdf;
};

/// 恢复源路径种子并消费阈值；initial 也必须调用此入口，不另建阈值随机流。
public TinyUniformSampleGenerator initReplaySampler(uint seed, ShiftSettings settings,
                                                    out float distanceThreshold,
                                                    out float roughnessThreshold)
{
    TinyUniformSampleGenerator sampler = rng_from_seed(seed);
    distanceThreshold = settings.nearFieldDistanceThreshold;
    roughnessThreshold = settings.specularRoughnessThreshold;
    if (settings.nearFieldDistanceSigma != 0.0)
        distanceThreshold = PathRISState::gaussrand(
            distanceThreshold, settings.nearFieldDistanceSigma, sampler.next2D());
    if (settings.specularRoughnessSigma != 0.0)
        roughnessThreshold = PathRISState::gaussrand(
            roughnessThreshold, settings.specularRoughnessSigma, sampler.next2D());
    return sampler;
}

/// 非 LightTileRIS 的共同消费顺序；关闭 NEE 或碰到 delta 时仍消费光源 padding。
/// RR 开启消费 11 维、关闭消费 10 维；只有 initial 使用 roulette 的存活决策。
public ReplayRandom bounceRandom(inout TinyUniformSampleGenerator sampler, bool useRussianRoulette)
{
    ReplayRandom result;
    result.uBSDFRIS = sampler.next1D();
    result.uNEERIS = sampler.next1D();
    result.neeSeed = sampler.state;
    result.lightSelector = sampler.next1D();
    result.light = sampler.next4D();
    result.roulette = useRussianRoulette ? sampler.next1D() : 0.0;
    float selector = sampler.next1D();
    result.bsdf = float3(sampler.next2D(), selector);
    return result;
}

/// 场景光源适配器使用联合 CDF；z 选择策略后须重映射为该策略内的均匀残差。
public float3 lightRandom(ReplayRandom random)
{
    return float3(random.light.xy, random.lightSelector);
}

/// NV 默认 Balance MIS；参数已经含各策略的样本数，不混用旧 PT 的 power heuristic。
public float lightMisWeight(float n0, float p0, float n1, float p1)
{
    float first = n0 * p0;
    float second = n1 * p1;
    return first > 0.0 ? first / (first + second) : 0.0;
}

/// 初采外部 RR 的存活率；存活时 throughput 除此值，sourcePdf 乘此值。
/// mixed-PSS 的重放去掉 roulette 维，仅跳过相同随机数，不再次补偿吞吐。
public float initialRouletteProbability(float3 throughput)
{
    return min(1.0, max(0.0, PathRISState::toScalar(throughput)));
}

/// 连续 BSDF 用立体角密度，delta 仅在 footprint 中用 NV 的有限大替代值。
public float replayScatterPdf(BsdfSample sample)
{
    return sample.delta ? 1.0e20 : sample.pdf;
}

/// 每段的几何量；两端点均为未偏移表面位置，visibility 的偏移不进入积分 measure。
public struct ReplaySegment
{
    public float geometryFactor;
    public float inverseGeometryFactor;
    public float rayFootprint;
};

/// 与 initial 的 setRcVertex 和 NEE/emitter 特例共同使用。
public ReplaySegment replaySegment(Surface previous, Surface current, float scatterPdf)
{
    float3 delta = current.point.position - previous.point.position;
    float distanceSquared = dot(delta, delta);
    float3 wo = -delta * rsqrt(distanceSquared);
    ReplaySegment result;
    result.geometryFactor = abs(dot(current.point.normal, wo)) / distanceSquared;
    result.inverseGeometryFactor = distanceSquared / abs(dot(previous.point.normal, wo));
    result.rayFootprint = 1.0 / (result.geometryFactor * scatterPdf);
    return result;
}

/// rcVertexLength=1 时直接使用目标 primary；其余路径只采样 prefix，不调用完整材质求值。
/// 无 reconnection 顶点（长度 255）时重放整个选中 BSDF-hit/environment 路径，返回真实目标
/// integrand。 pathLength 是最后 scatter 的零基索引；primary scatter 为
/// 0。无效或不可逆路径返回零贡献。
public ReconnectionData traceReplayedPath<Scene : IRestirScene>(
    inout Scene scene, Surface primary, PathReservoir source, float primaryRayFootprint,
    ShiftSettings settings, bool useRussianRoulette, bool useNEE, bool useMIS, bool previous)
{
    ReconnectionData result = {};
    if (!primary.hit.isValid() || source.weight == 0.0)
        return result;
    uint rcLength = source.pathFlags.rcVertexLength();
    uint pathLength = source.pathFlags.pathLength();
    bool shouldConnect = rcLength <= pathLength + 1;
    float3 primaryThroughput = exp(-primary.incident.yzw * primary.distance);
    if (shouldConnect && rcLength == 1)
    {
        if (!primary.hasNonDeltaLobes)
            return result;
        result.rcPrevHit = primary.hit;
        result.rcPrevWo = primary.wo;
        result.rcPrevIncident = primary.incident;
        result.pathThroughput = primaryThroughput;
        result.rcFootprintFactor = -1.0;
        return result;
    }
    if (source.pathFlags.lastVertexNEE() && !shouldConnect)
        return result;

    float distanceThreshold;
    float roughnessThreshold;
    TinyUniformSampleGenerator sampler =
        initReplaySampler(source.initRandomSeed, settings, distanceThreshold, roughnessThreshold);
    Surface surface = primary;
    float3 throughput = primaryThroughput;
    bool lastRough = false;
    ReplaySegment incoming = {};
    for (uint depth = 0; depth <= pathLength; ++depth)
    {
        ReplayRandom random = bounceRandom(sampler, useRussianRoulette);
        BsdfSample scatter = scene.sampleBsdf(surface, random.bsdf);
        if (scatter.pdf <= 0.0 || all(scatter.weight == 0.0) ||
            any(isnan(scatter.weight) | isinf(scatter.weight)) || isinf(scatter.pdf))
            return result;
        float scatterPdf = replayScatterPdf(scatter);
        bool rough = !scatter.delta && scene.isSampleRough(surface, scatter.component, scatterPdf,
                                                           roughnessThreshold);

        // 在到达源 reconnection 前不能出现更早可连接顶点，否则映射不能逆回同一 prefix。
        if (depth > 0 && depth < rcLength && lastRough &&
            min(incoming.rayFootprint, incoming.inverseGeometryFactor / scatterPdf) >=
                primaryRayFootprint * distanceThreshold)
            return result;

        throughput *= scatter.weight;
        float3 origin = geometry::offset_origin(surface.point, scatter.direction);
        HitInfo hit = {};
        float distance;
        bool found = scene.trace(origin, scatter.direction, previous, hit, distance);
        if (!found)
        {
            if (shouldConnect || depth != pathLength || source.pathFlags.lightType() != LIGHT_ENV ||
                rough)
                return result;
            float3 radiance =
                scene.environment(scatter.direction, surface.point.position, previous);
            radiance *= exp(-scatter.medium.yzw * scene.environmentDistance());
            float lightPdf = scene.lightPdf(surface, hit, scatter.direction, LIGHT_ENV, previous);
            float mis = useNEE && useMIS && !scatter.delta
                            ? lightMisWeight(1.0, scatter.pdf, 1.0, lightPdf)
                            : 1.0;
            result.pathThroughput = throughput * radiance * mis;
            return result;
        }

        throughput *= exp(-scatter.medium.yzw * distance);
        uint nextDepth = depth + 1;
        if (nextDepth == pathLength + 1 && !source.pathFlags.lastVertexNEE())
        {
            if (shouldConnect || source.pathFlags.lightType() != LIGHT_EMISSIVE)
                return result;
            Surface endpoint = {};
            endpoint.hit = hit;
            endpoint.wo = -scatter.direction;
            EmitterEvaluation emitter =
                scene.evaluateEmitter(endpoint, surface.point.position, previous);
            if (!any(emitter.emission > 0.0))
                return result;
            endpoint.point.position = emitter.position;
            endpoint.point.normal = emitter.normal;
            ReplaySegment emitterSegment = replaySegment(surface, endpoint, scatterPdf);
            bool emitterCouldReconnect =
                min(emitterSegment.rayFootprint,
                    emitterSegment.inverseGeometryFactor / (0.5 / 3.14159265358979323846)) >=
                    primaryRayFootprint * distanceThreshold &&
                (!surface.supportSimpleRoughnessThreshold || rough);
            if (nextDepth < rcLength && emitterCouldReconnect)
                return result;
            float lightPdf = emitter.pdf;
            float mis = useNEE && useMIS && !scatter.delta
                            ? lightMisWeight(1.0, scatter.pdf, 1.0, lightPdf)
                            : 1.0;
            result.pathThroughput = throughput * emitter.emission * mis;
            return result;
        }
        Surface next = scene.loadSurface(hit, -scatter.direction, previous);
        scene.setIncident(next, scatter.medium);
        incoming = replaySegment(surface, next, scatterPdf);
        if (shouldConnect && nextDepth == rcLength - 1)
        {
            if (!next.hasNonDeltaLobes)
                return result;
            result.rcPrevHit = hit;
            result.rcPrevWo = next.wo;
            result.rcPrevIncident = next.incident;
            result.pathThroughput = throughput;
            result.rcFootprintFactor =
                !rough || incoming.rayFootprint < primaryRayFootprint * distanceThreshold
                    ? -1.0
                    : incoming.inverseGeometryFactor;
            return result;
        }
        surface = next;
        lastRough = rough;
    }
    return result;
}
}
