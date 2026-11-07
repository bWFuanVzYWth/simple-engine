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
/// NV Hybrid 与独立 RR sourcePdf 契约见 replay；不改变上层配置的反弹上限。
module initial;

import restir_pt.types;
import restir_pt.scene;
import restir_pt.rng;
__exported import restir_pt.initial_random;
import restir_pt.replay;
import geometry.ray;

namespace restir_pt
{
/// reservoir 估计主表面之后的路径；residual 保存默认 point 域之外的主发光/主环境。
public struct InitialResult
{
    public PathReservoir reservoir;
    public float3 residual;
};

/// 初采追踪上下文只属于当前raygen；trace发布的新命中供下一次shade消费。
/// 已知表面的shift/sample/evaluate不经过此入口，继续由IRestirScene管理。
public interface IInitialTrace<Scene : IRestirScene, State>
{
    [mutating]
    bool trace(inout Scene scene, inout State state, float3 origin, float3 direction,
               out HitInfo hit, out float distance);
    /// 当前新命中的表面；fromOrigin区分观察位置和已知wo，不能用于历史或光端点。
    Surface load(inout Scene scene, inout State state, HitInfo hit, float3 view, bool fromOrigin);
    [mutating]
    InitialVertex shade(inout Scene scene, inout State state, Surface surface,
                        InitialRequest request);
};

/// 兼容原始TraceRay/callable实现；无额外状态或运行时分支。
public struct InitialTrace<Scene : IRestirScene> : IInitialTrace<Scene, uint>
{
    [mutating]
    public bool trace(inout Scene scene, inout uint state, float3 origin, float3 direction,
                      out HitInfo hit, out float distance)
    {
        return scene.trace(origin, direction, false, hit, distance);
    }
    public Surface load(inout Scene scene, inout uint state, HitInfo hit, float3 view,
                        bool fromOrigin)
    {
        return fromOrigin ? scene.loadSurfaceFromOrigin(hit, view, false)
                          : scene.loadSurface(hit, view, false);
    }
    [mutating]
    public InitialVertex shade(inout Scene scene, inout uint state, Surface surface,
                               InitialRequest request)
    {
        return scene.shadeInitialVertex(surface, request);
    }
};

LastVertexState initialLastVertex(BsdfSample sample, Surface surface, bool rough,
                                  ReplaySegment segment, float primaryFootprint,
                                  float distanceThreshold)
{
    LastVertexState result;
    result.Init(sample.delta, sample.transmission, sample.component, surface.supportBSDFIndexing,
                rough, segment.rayFootprint >= primaryFootprint * distanceThreshold);
    return result;
}

bool initialCanConnect(ShiftSettings settings, uint depth, uint rcLength, ReplaySegment segment,
                       float outgoingPdf, bool previousRough, float primaryFootprint,
                       float distanceThreshold)
{
    return settings.shiftMapping == 0
               ? depth == 1
               : rcLength == MAXIMUM_RC_LENGTH && depth >= 1 && previousRough &&
                     min(segment.rayFootprint, segment.inverseGeometryFactor / outgoingPdf) >=
                         primaryFootprint * distanceThreshold;
}

/// 使用已保存 primaryHit；maxSurfaceBounces 须不超过254，与8bit路径长度保持一致。
/// primaryHit、初始相机/介质与当前scene须属于同一代；所有材质和求交均为实际场景操作。
/// 每个顶点合并eval/发光；预算耗尽后不采样散射，末顶点NEE不含BSDF竞争。
/// LightSamples=1保留联合灯分布；=2分别采u.z的两个半区，各为一个加性灯族，
/// PDF不含族选择概率。第二族使用原padding维，不改变散射/RR及重放的随机身份。
/// 完整材质阶段一次产生可见NEE候选；调度器不重采灯、不重算材质支持或介质。
public InitialResult sampleInitialPath<Scene : IRestirScene, let LightSamples : uint = 1>(
    inout Scene scene, uint2 pixel, HitInfo primaryHit, uint initialSeed, uint maxSurfaceBounces,
    ShiftSettings settings, bool useRussianRoulette, bool useNEE, bool useMIS)
{
    InitialTrace<Scene> tracer;
    uint state = 0;
    return sampleInitialPathTraced<Scene, uint, InitialTrace<Scene>, LightSamples>(
        scene, tracer, state, pixel, primaryHit, initialSeed, maxSurfaceBounces, settings,
        useRussianRoulette, useNEE, useMIS);
}

/// primaryHit须由同一个tracer.trace产生；每次新命中仅在下一顶点着色前有效。
/// tracer拥有硬件命中状态，算法不复制或持久化它；其余随机域、RIS及可见性不变。
public InitialResult sampleInitialPathTraced<
    Scene : IRestirScene, State, Trace : IInitialTrace<Scene, State>, let LightSamples : uint = 1>(
    inout Scene scene, inout Trace tracer, inout State state, uint2 pixel, HitInfo primaryHit,
    uint initialSeed, uint maxSurfaceBounces, ShiftSettings settings, bool useRussianRoulette,
    bool useNEE, bool useMIS)
{
    static_assert(LightSamples == 1 || LightSamples == 2,
                  "initial NEE supports one or two families");
    InitialResult result = {};
    result.reservoir.init();
    if (!primaryHit.isValid())
    {
        result.residual =
            scene.environment(scene.initialDirection(pixel), scene.initialOrigin(pixel), false) *
            exp(-scene.initialMedium(pixel).yzw * scene.environmentDistance());
        return result;
    }
    Surface surface = tracer.load(scene, state, primaryHit, scene.initialOrigin(pixel), true);
    // primary 的物理生产者是实际相机射线；不从有限精度重建点重新推导另一条方向。
    surface.wo = -scene.initialDirection(pixel);
    scene.setIncident(surface, scene.initialMedium(pixel));
    float3 throughput = exp(-surface.incident.yzw * surface.distance);
    float primaryFootprint = scene.primaryRayFootprint(surface, pixel, false);
    float distanceThreshold;
    float roughnessThreshold;
    TinyUniformSampleGenerator sampler =
        initReplaySampler(initialSeed, settings, distanceThreshold, roughnessThreshold);
    PathRISState ris = {};
    ris.init();
    PathReconnectionInfo rc = {};
    rc.pathFlags.insertRcVertexLength(MAXIMUM_RC_LENGTH);
    rc.rcThroughput = 1.0;
    rc.initRandomSeed = initialSeed;
    float sourcePdf = 1.0;
    float previousScatterPdf = 1.0;
    float previousFullPdf = 1.0;
    bool previousRough = false;
    BsdfSample previousScatter = {};
    Surface previousSurface = {};
    ReplaySegment segment = {};
    float3 previousSegmentTransmittance = 1.0;
    for (uint depth = 0; depth <= maxSurfaceBounces; ++depth)
    {
        uint bounceSeed = sampler.state;
        sampler.state = useRussianRoulette ? initialRandomState<11>(bounceSeed)
                                           : initialRandomState<10>(bounceSeed);
        float rouletteProbability =
            useRussianRoulette ? initialRouletteProbability(throughput) : 1.0;
        bool rouletteKilled =
            useRussianRoulette && (rouletteProbability == 0.0 ||
                                   initialRandomValue<8>(bounceSeed) >= rouletteProbability);
        bool continuation = depth < maxSurfaceBounces;
        InitialRequest request = {
            bounceSeed,
            (continuation && !rouletteKilled ? INITIAL_SAMPLE : 0u) | (useNEE ? INITIAL_NEE : 0u) |
                (continuation && useMIS ? INITIAL_MIS : 0u) |
                (useRussianRoulette ? INITIAL_RR : 0u) |
                (LightSamples == 2 ? INITIAL_TWO_LIGHTS : 0u) | (depth == 0 ? INITIAL_PRIMARY : 0u),
            previousSurface.point.position,
            throughput,
            ris.weight,
            useNEE && useMIS && !previousScatter.delta ? previousFullPdf : -1.0
        };
        InitialVertex shading = tracer.shade(scene, state, surface, request);
        surface.emission = shading.emission;
        ris.weight = shading.weight;

        LastVertexState last = initialLastVertex(previousScatter, previousSurface, previousRough,
                                                 segment, primaryFootprint, distanceThreshold);
        if (depth == 0)
        {
            result.residual = throughput * surface.emission;
        }
        else if (any(surface.emission > 0.0))
        {
            float lightPdf = shading.emissionPdf;
            float mis = useNEE && useMIS && !previousScatter.delta
                            ? lightMisWeight(1.0, previousFullPdf, 1.0, lightPdf)
                            : 1.0;
            float3 cachedEmission = surface.emission;
            if (depth - 1 == rc.pathFlags.rcVertexLength())
                cachedEmission *= previousSegmentTransmittance;
            ris.storeBSDFHitCandidate(rc, depth - 1, throughput * mis * surface.emission,
                                      cachedEmission, mis * surface.emission, sourcePdf, lightPdf,
                                      LIGHT_EMISSIVE);
            if (rc.pathFlags.rcVertexLength() == MAXIMUM_RC_LENGTH &&
                (settings.shiftMapping == 1 || depth == 1))
            {
                bool canConnect =
                    settings.shiftMapping == 0 ||
                    min(segment.rayFootprint,
                        segment.inverseGeometryFactor / (0.5 / 3.14159265358979323846)) >=
                            primaryFootprint * distanceThreshold &&
                        (!previousSurface.supportSimpleRoughnessThreshold || previousRough);
                if (canConnect)
                {
                    ris.integrand *= previousScatterPdf;
                    ris.markAsRcVertex(depth, surface.hit, last, lightPdf * segment.geometryFactor,
                                       LIGHT_EMISSIVE, surface.emission, float3(0.0),
                                       segment.geometryFactor);
                }
            }
        }
        if (shading.direct.pdf > 0.0)
        {
            InitialDirect light = shading.direct;
            uint lightType = light.lightType & ~INITIAL_POINT_LIGHT;
            bool canConnect = initialCanConnect(settings, depth, rc.pathFlags.rcVertexLength(),
                                                segment, light.bsdfPdf, previousRough,
                                                primaryFootprint, distanceThreshold);
            float3 cachedRadiance = canConnect ? light.attenuatedRadiance : light.radiance;
            ris.storeNEECandidate(rc, canConnect, depth, throughput * light.reflected,
                                  light.direction, cachedRadiance, light.reflected, sourcePdf,
                                  light.pdf, lightType);
            ris.pathFlags.insertTerminalNEE(!continuation);
            if (canConnect)
            {
                ris.reconnectionHit = surface.hit;
                ris.reconnectionJacobian = segment.geometryFactor;
                ris.integrand *= previousScatterPdf * light.pdf;
                ris.rcNextVertexBounceSeed = initialRandomState<2>(bounceSeed);
                ris.pathFlags.tryInsertBounceTypesBeforeRcVertex(last.data & 0x3fu, true);
            }
            if (!canConnect && rc.pathFlags.rcVertexLength() == MAXIMUM_RC_LENGTH)
            {
                ris.reconnectionHit = light.hit;
                ris.reconnectionJacobian = 1.0;
                ris.reconnectionIncidentRadiance = light.radiance;
                ris.reconnectionWi = light.direction;
                ris.integrand *= light.pdf;
                ris.pathFlags.insertRcVertexLength(ris.pathFlags.pathLength() + 1);
                ris.pathFlags.insertIsTransmissionEvent(
                    dot(surface.point.normal, surface.wo) *
                            dot(surface.point.normal, light.direction) <
                        0.0,
                    true);
                ris.pathFlags.insertSupportBSDFIndexingBeforeRcVertex(surface.supportBSDFIndexing);
                if (light.hit.isValid())
                {
                    if ((light.lightType & INITIAL_POINT_LIGHT) != 0)
                        ris.reconnectionIncidentRadiance /= light.geometryFactor;
                    else
                    {
                        ris.reconnectionJacobian *= light.geometryFactor;
                        ris.lightPdf *= light.geometryFactor;
                    }
                }
            }
        }

        if (!continuation)
            break;

        if (useRussianRoulette)
        {
            if (rouletteKilled)
                break;
            throughput /= rouletteProbability;
            sourcePdf *= rouletteProbability;
        }
        BsdfSample scatter = shading.sample;
        if (scatter.pdf <= 0.0 || all(scatter.weight == 0.0) ||
            any(isnan(scatter.weight) | isinf(scatter.weight)) || isinf(scatter.pdf))
            break;
        float scatterPdf = replayScatterPdf(scatter);
        bool rough = !scatter.delta && scene.isSampleRough(surface, scatter.component, scatterPdf,
                                                           roughnessThreshold);
        float componentPdf = scatter.delta                 ? 1.0
                             : surface.supportBSDFIndexing ? scatter.componentPdf
                                                           : scatter.pdf;
        throughput *= scatter.weight;
        rc.rcThroughput *= scatter.weight;
        if (initialCanConnect(settings, depth, rc.pathFlags.rcVertexLength(), segment, scatterPdf,
                              previousRough, primaryFootprint, distanceThreshold))
        {
            rc.rcNextVertexBounceSeed = sampler.state;
            rc.reconnectionWi = scatter.direction;
            rc.pathFlags.insertRcVertexLength(depth);
            rc.reconnectionHit = surface.hit;
            rc.pathFlags.tryInsertBounceTypesBeforeRcVertex(last.data & 0x3fu, true);
            rc.reconnectionJacobian = segment.geometryFactor;
            sourcePdf *= previousScatterPdf * componentPdf;
            rc.pathFlags.tryInsertBounceTypesAfterRcVertex(
                uint(scatter.delta) | (uint(scatter.transmission) << 1) | (scatter.component << 2),
                true);
            rc.rcThroughput = 1.0;
        }
        float3 origin = geometry::offset_origin(surface.point, scatter.direction);
        HitInfo hit = {};
        float distance;
        bool found = tracer.trace(scene, state, origin, scatter.direction, hit, distance);
        if (!found)
        {
            float uBSDFRIS = sampler.next1D();
            float3 radiance = scene.environment(scatter.direction, surface.point.position, false);
            float3 environmentTransmittance =
                exp(-scatter.medium.yzw * scene.environmentDistance());
            float lightPdf = scene.lightPdf(surface, hit, scatter.direction, LIGHT_ENV, false);
            float mis = useNEE && useMIS && !scatter.delta
                            ? lightMisWeight(1.0, scatter.pdf, 1.0, lightPdf)
                            : 1.0;
            LastVertexState envLast;
            envLast.Init(scatter.delta, scatter.transmission, scatter.component,
                         surface.supportBSDFIndexing, rough, true);
            float3 cachedRadiance = depth == rc.pathFlags.rcVertexLength()
                                        ? radiance * environmentTransmittance
                                        : radiance;
            bool selected = ris.addBSDFHitCandidate(
                uBSDFRIS, rc, depth, throughput * environmentTransmittance * mis * radiance,
                scatter.direction, cachedRadiance, environmentTransmittance * mis * radiance,
                sourcePdf, lightPdf, LIGHT_ENV);
            if (selected && rc.pathFlags.rcVertexLength() == MAXIMUM_RC_LENGTH &&
                (settings.shiftMapping == 1 || depth == 0) && (rough || settings.shiftMapping == 0))
            {
                ris.integrand *= componentPdf;
                ris.markAsRcVertex(depth + 1, hit, envLast, lightPdf, LIGHT_ENV, radiance,
                                   scatter.direction, 1.0);
            }
            break;
        }
        float3 attenuation = exp(-scatter.medium.yzw * distance);
        throughput *= attenuation;
        rc.rcThroughput *= attenuation;
        previousSegmentTransmittance = attenuation;
        Surface next = tracer.load(scene, state, hit, -scatter.direction, false);
        scene.setIncident(next, scatter.medium);
        segment = replaySegment(surface, next, scatterPdf);
        previousSurface = surface;
        previousScatter = scatter;
        previousScatterPdf = componentPdf;
        previousFullPdf = scatter.pdf;
        previousRough = rough;
        surface = next;
    }
    result.reservoir = PathReservoir::createFromRisState(ris, initialSeed);
    result.reservoir.finalizeReservoirWeight();
    return result;
}
}
