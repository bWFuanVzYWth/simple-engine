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

// 只保留bounce起点；LCG跳跃在模2^32上与逐维next完全相同。
uint initialRandomState<let Dimension : uint>(uint seed)
{
    uint multiplier = 1u;
    uint increment = 0u;
    [unroll]
    for (uint dimension = 0; dimension < Dimension; ++dimension)
    {
        multiplier *= 1664525u;
        increment = 1664525u * increment + 1013904223u;
    }
    return multiplier * seed + increment;
}

float initialRandomValue<let Dimension : uint>(uint seed)
{
    return float(initialRandomState<Dimension>(seed) >> 8) * 0x1p-24f;
}

float3 initialLightRandom(uint seed)
{
    return float3(initialRandomValue<4>(seed), initialRandomValue<5>(seed),
                  initialRandomValue<3>(seed));
}

float3 initialBsdfRandom(uint seed, bool useRussianRoulette)
{
    uint selector = useRussianRoulette ? initialRandomState<9>(seed) : initialRandomState<8>(seed);
    TinyUniformSampleGenerator sampler = rng_from_seed(selector);
    float2 direction = sampler.next2D();
    return float3(direction, float(selector >> 8) * 0x1p-24f);
}

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
/// sample/eval/发光在非末端顶点合并为一次 shadeInitial，重放只调用独立 sample 窄入口。
/// 本实验要求sampleLight对相同Surface/u为纯函数，shadeInitial不改变灯采样状态。
public InitialResult sampleInitialPath<Scene : IRestirScene>(
    inout Scene scene, uint2 pixel, HitInfo primaryHit, uint initialSeed, uint maxSurfaceBounces,
    ShiftSettings settings, bool useRussianRoulette, bool useNEE, bool useMIS)
{
    InitialResult result = {};
    result.reservoir.init();
    if (!primaryHit.isValid())
    {
        result.residual =
            scene.environment(scene.initialDirection(pixel), scene.initialOrigin(pixel), false) *
            exp(-scene.initialMedium(pixel).yzw * scene.environmentDistance());
        return result;
    }
    Surface surface = scene.loadSurfaceFromOrigin(primaryHit, scene.initialOrigin(pixel), false);
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
        bool sampleNEE = depth < maxSurfaceBounces && useNEE && surface.hasNonDeltaLobes;
        float3 neeDirection = surface.wo;
        if (sampleNEE)
        {
            LightSample light = scene.sampleLight(surface, initialLightRandom(bounceSeed));
            neeDirection = light.pdf > 0.0 ? light.direction : surface.wo;
        }
        InitialShading shading = {};
        if (depth < maxSurfaceBounces)
            shading = scene.shadeInitial(
                surface,
                rouletteKilled ? float3(0.0) : initialBsdfRandom(bounceSeed, useRussianRoulette),
                neeDirection, !rouletteKilled);
        // sampleLight为当前scene的纯函数；同seed/同完整Surface重算，结束前不覆盖emission。
        LightSample light = {};
        if (sampleNEE)
            light = scene.sampleLight(surface, initialLightRandom(bounceSeed));
        if (depth < maxSurfaceBounces)
            surface.emission = shading.emission;

        LastVertexState last = initialLastVertex(previousScatter, previousSurface, previousRough,
                                                 segment, primaryFootprint, distanceThreshold);
        if (depth == 0)
        {
            result.residual = throughput * surface.emission;
        }
        else if (any(surface.emission > 0.0))
        {
            float lightPdf =
                scene.lightPdf(previousSurface, surface.hit, -surface.wo, LIGHT_EMISSIVE, false);
            float mis = useNEE && useMIS && !previousScatter.delta
                            ? lightMisWeight(1.0, previousFullPdf, 1.0, lightPdf)
                            : 1.0;
            float3 cachedEmission = surface.emission;
            if (depth - 1 == rc.pathFlags.rcVertexLength())
                cachedEmission *= previousSegmentTransmittance;
            bool selected = ris.addBSDFHitCandidate(
                initialRandomValue<1>(bounceSeed), rc, depth - 1,
                throughput * mis * surface.emission, -surface.wo, cachedEmission,
                mis * surface.emission, sourcePdf, lightPdf, LIGHT_EMISSIVE);
            if (selected && rc.pathFlags.rcVertexLength() == MAXIMUM_RC_LENGTH &&
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
        if (depth == maxSurfaceBounces)
            break;

        if (light.pdf > 0.0 && any(light.radiance > 0.0))
        {
            float lightMis = useMIS && light.lightType != LIGHT_ANALYTIC
                                 ? lightMisWeight(1.0, light.pdf, 1.0, shading.nee.pdf)
                                 : 1.0;
            float3 lightTransmittance =
                scene.transmittance(surface, light.direction, light.distance);
            float3 reflected =
                shading.nee.value * light.radiance * lightTransmittance * (lightMis / light.pdf);
            Surface target = {};
            if (light.hit.isValid())
                target = scene.loadSurfaceFromOrigin(light.hit, surface.point.position, false);
            bool visible = false;
            if (any(reflected > 0.0))
                visible = light.hit.isValid()
                              ? scene.visible(surface, target, false)
                              : scene.visibleDirection(surface, light.direction, false);
            if (visible)
            {
                bool canConnect = initialCanConnect(settings, depth, rc.pathFlags.rcVertexLength(),
                                                    segment, shading.nee.pdf, previousRough,
                                                    primaryFootprint, distanceThreshold);
                float3 cachedRadiance =
                    canConnect ? light.radiance * lightTransmittance : light.radiance;
                bool selected =
                    ris.addNEECandidate(initialRandomValue<2>(bounceSeed), rc, canConnect, depth,
                                        throughput * reflected, light.direction, cachedRadiance,
                                        reflected, sourcePdf, light.pdf, light.lightType);
                if (selected && canConnect)
                {
                    ris.reconnectionHit = surface.hit;
                    ris.reconnectionJacobian = segment.geometryFactor;
                    ris.integrand *= previousScatterPdf * light.pdf;
                    ris.rcNextVertexBounceSeed = initialRandomState<2>(bounceSeed);
                    ris.pathFlags.tryInsertBounceTypesBeforeRcVertex(last.data & 0x3fu, true);
                }
                if (selected && !canConnect && rc.pathFlags.rcVertexLength() == MAXIMUM_RC_LENGTH)
                {
                    // 无普通 rc 的 NEE 必须保存真实光端点，不能缩成旧方向的颜色缓存。
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
                    ris.pathFlags.insertSupportBSDFIndexingBeforeRcVertex(
                        surface.supportBSDFIndexing);
                    if (light.hit.isValid())
                    {
                        bool pointLight =
                            light.lightType == LIGHT_ANALYTIC && scene.isPointLight(light.hit);
                        float geometryFactor =
                            (pointLight ? 1.0 : abs(dot(target.point.normal, -light.direction))) /
                            (light.distance * light.distance);
                        if (pointLight)
                        {
                            ris.reconnectionIncidentRadiance /= geometryFactor;
                        }
                        else
                        {
                            ris.reconnectionJacobian *= geometryFactor;
                            ris.lightPdf *= geometryFactor;
                        }
                    }
                }
            }
        }

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
        bool found = scene.trace(origin, scatter.direction, false, hit, distance);
        if (!found)
        {
            float uBSDFRIS = sampler.next1D();
            float3 radiance = scene.environment(scatter.direction, surface.point.position, false);
            float3 environmentTransmittance =
                scene.transmittance(surface, scatter.direction, scene.environmentDistance());
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
        Surface next = scene.loadSurface(hit, -scatter.direction, false);
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
