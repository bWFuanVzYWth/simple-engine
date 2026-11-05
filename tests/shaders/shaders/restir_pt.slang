/// 解析 Lambert 场景 oracle；路径追踪、shift 与 GRIS 直接实例化生产泛型算法。
module restir_pt_test;

import restir_pt.types;
import restir_pt.rng;
import restir_pt.scene;
import restir_pt.initial;
import restir_pt.shift;
import restir_pt.reservoir;
import restir_pt.resampling;

using namespace restir_pt;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;
static const float PI = 3.14159265358979323846;

/// mode 2/3 为 z=2 无限发光平面，其 NEE 只采样边长4的子区域；其余为常量环境。
/// mode 7/8 为 z=0/1 的 Lambert 发光腔体，禁用NEE/MIS，最多10次散射。
/// variant 改变 primary 法线或横向位置，使双向 shift 的支持域/PDF 与源域不同。
public struct LambertScene : IRestirScene
{
    public uint mode;
    public uint variant;
    public float3 albedo;
    public float radiance;
    public float extinction;
    public float angle;

    public bool finiteLight()
    {
        return mode == 2 || mode == 3;
    }
    public bool cavity()
    {
        return mode == 7 || mode == 8;
    }
    public bool useNEE()
    {
        return mode != 1 && !cavity();
    }
    public bool useMIS()
    {
        return !cavity();
    }
    public float3 normal()
    {
        float a = finiteLight() || cavity() || mode == 9 || variant == 0 ? 0.0 : angle;
        return float3(sin(a), 0.0, cos(a));
    }
    public float3 position()
    {
        return finiteLight() || cavity() || mode == 6 ? float3(float(variant) * 0.7, 0.0, 0.0)
                                                      : float3(0.0);
    }
    public HitInfo primaryHit()
    {
        HitInfo hit;
        hit.data = uint4(mode == 5 ? 0u : 1u, asuint(position()));
        return hit;
    }
    public Surface loadSurface(HitInfo hit, float3 wo, bool previous)
    {
        Surface result = {};
        result.hit = hit;
        result.point.position = asfloat(hit.data.yzw);
        result.point.normal = hit.data.x == 1 ? normal() : float3(0.0, 0.0, -1.0);
        result.wo = wo;
        result.emission = cavity() || hit.data.x == 2 ? float3(radiance) : float3(0.0);
        result.incident = float4(1.0, extinction, extinction, extinction);
        result.transmitted = result.incident;
        // 确定性准入策略使Hybrid只允许下面作为RC，深路径必须先重放到上面。
        result.roughness = cavity() && hit.data.x == 1 ? 0.0 : 1.0;
        result.supportSimpleRoughnessThreshold = true;
        result.hasNonDeltaLobes = cavity() || hit.data.x == 1;
        result.frontFacing = dot(result.point.normal, wo) > 0.0;
        return result;
    }
    public Surface loadSurfaceFromOrigin(HitInfo hit, float3 origin, bool previous)
    {
        Surface result = loadSurface(hit, float3(0.0), previous);
        float3 delta = origin - result.point.position;
        result.distance = length(delta);
        result.wo = delta / result.distance;
        result.frontFacing = dot(result.point.normal, result.wo) > 0.0;
        return result;
    }
    public void setIncident(inout Surface surface, float4 medium)
    {
        surface.incident = medium;
    }
    public bool trace(float3 origin, float3 direction, bool previous, out HitInfo hit,
                      out float distance)
    {
        hit = {};
        distance = environmentDistance();
        if (cavity())
        {
            bool upper = direction.z > 0.0;
            distance = ((upper ? 1.0 : 0.0) - origin.z) / direction.z;
            hit.data = uint4(upper ? 2u : 1u, asuint(origin + distance * direction));
            return distance > 0.0;
        }
        if (!finiteLight() || direction.z <= 0.0)
            return false;
        distance = (2.0 - origin.z) / direction.z;
        hit.data = uint4(2u, asuint(origin + distance * direction));
        return distance > 0.0;
    }
    public bool visible(Surface source, Surface target, bool previous)
    {
        return true;
    }
    public bool visibleDirection(Surface source, float3 direction, bool previous)
    {
        return true;
    }
    public float3 transmittance(Surface source, float3 direction, float distance)
    {
        return float3(exp(-extinction * distance));
    }
    public float environmentDistance()
    {
        return 2.0;
    }
    public BsdfSample sampleBsdf(Surface surface, float3 u)
    {
        float3 n = surface.point.normal;
        float3 tangent = float3(n.z, 0.0, -n.x);
        float r = sqrt(u.x);
        float phi = 2.0 * PI * u.y;
        BsdfSample result = {};
        result.direction = tangent * (r * cos(phi)) + float3(0.0, r * sin(phi), 0.0) +
                           n * sqrt(max(0.0, 1.0 - u.x));
        result.weight = albedo;
        result.pdf = max(0.0, dot(n, result.direction)) / PI;
        result.componentPdf = result.pdf;
        result.medium = surface.incident;
        return result;
    }
    public InitialShading shadeInitial(Surface surface, float3 u, float3 direction,
                                       float3 secondDirection, bool sampleContinuation)
    {
        BsdfSample sample = {};
        if (sampleContinuation)
            sample = sampleBsdf(surface, u);
        return { sample, evaluateBsdf(surface, direction, -1), surface.emission,
                 evaluateBsdf(surface, secondDirection, -1) };
    }
    public BsdfEvaluation evaluateBsdf(Surface surface, float3 direction, int component)
    {
        float pdf = max(0.0, dot(surface.point.normal, direction)) / PI;
        return { albedo * pdf, pdf };
    }
    public bool isSampleRough(Surface surface, uint component, float pdf, float threshold)
    {
        return surface.roughness >= threshold;
    }
    public LightSample sampleLight(Surface surface, float3 u)
    {
        LightSample result = {};
        result.radiance = float3(radiance);
        if (finiteLight())
        {
            float3 point = float3(4.0 * u.xy - 2.0, 2.0);
            float3 delta = point - surface.point.position;
            result.distance = length(delta);
            result.direction = delta / result.distance;
            result.hit.data = uint4(2u, asuint(point));
            result.pdf = result.distance * result.distance / (16.0 * result.direction.z);
            result.lightType = LIGHT_EMISSIVE;
        }
        else
        {
            float cosine = u.x;
            float sine = sqrt(1.0 - cosine * cosine);
            float phi = 2.0 * PI * u.y;
            if (mode == 9)
                phi = PI * (u.y + float(u.z >= 0.5));
            float3 n = surface.point.normal;
            result.direction = float3(n.z, 0.0, -n.x) * (sine * cos(phi)) +
                               float3(0.0, sine * sin(phi), 0.0) + n * cosine;
            result.distance = environmentDistance();
            result.pdf = (mode == 9 ? 1.0 : 0.5) / PI;
            result.lightType = LIGHT_ENV;
            result.radiance = environment(result.direction, surface.point.position, false);
        }
        return result;
    }
    public float lightPdf(Surface surface, HitInfo light, float3 direction, uint lightType,
                          bool previous)
    {
        if (cavity())
            return 0.0;
        if (!finiteLight())
            return dot(surface.point.normal, direction) > 0.0 ? (mode == 9 ? 1.0 : 0.5) / PI : 0.0;
        float3 point = asfloat(light.data.yzw);
        if (any(abs(point.xy) > 2.0) || direction.z <= 0.0)
            return 0.0;
        float3 delta = point - surface.point.position;
        return dot(delta, delta) / (16.0 * direction.z);
    }
    public float3 environment(float3 direction, float3 origin, bool previous)
    {
        if (cavity())
            return 0.0;
        return float3(radiance * (mode == 6 ? 1.0 + 0.5 * origin.x : 1.0));
    }
    public bool isPointLight(HitInfo hit)
    {
        return false;
    }
    public float3 initialOrigin(uint2 pixel)
    {
        return position() + normal() * (cavity() ? 0.5 : 1.0);
    }
    public float3 initialDirection(uint2 pixel)
    {
        return -normal();
    }
    public float4 initialMedium(uint2 pixel)
    {
        return float4(1.0, extinction, extinction, extinction);
    }
    public float primaryRayFootprint(Surface surface, uint2 pixel, bool previous)
    {
        return 1.0;
    }
}

LambertScene testScene(uint variant)
{
    LambertScene scene;
    scene.mode = inputs[0].x;
    scene.variant = variant;
    float4 a = asfloat(inputs[1]);
    scene.extinction = a.z;
    scene.angle = a.w;
    float4 b = asfloat(inputs[2]);
    scene.albedo = b.xyz;
    scene.radiance = b.w;
    return scene;
}

ShiftSettings testSettings()
{
    ShiftSettings settings = {};
    settings.nearFieldDistanceSigma = 0.0;
    settings.specularRoughnessSigma = 0.0;
    settings.specularRoughnessThreshold = asfloat(inputs[1].x);
    settings.nearFieldDistanceThreshold = asfloat(inputs[1].y);
    return settings;
}

PrecomputedShiftData shifted(inout LambertScene destination, PathReservoir source,
                             ShiftSettings settings)
{
    Surface primary = destination.loadSurfaceFromOrigin(destination.primaryHit(),
                                                        destination.initialOrigin(0), false);
    PrecomputedShiftData result = {};
    result.pathThroughput =
        shiftPath(destination, primary, source, 1.0, settings, true, destination.useNEE(),
                  destination.useMIS(), false, result.Jacobian);
    result.reconnectionJacobian = source.reconnectionJacobian;
    return result;
}

/// 每线程一个独立 ensemble；输出初采、恒等移位误差、temporal、spatial 与分支计数。
[numthreads(64, 1, 1)]
void restir_mean_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x >= inputs[0].y)
        return;
    LambertScene canonicalScene = testScene(0);
    LambertScene neighborScene = testScene(1);
    ShiftSettings settings = testSettings();
    TinyUniformSampleGenerator canonicalRng = rng_for_pixel(uint2(id.x, 0), 1);
    TinyUniformSampleGenerator neighborRng = rng_for_pixel(uint2(id.x, 1), 1);
    TinyUniformSampleGenerator reuseRng = rng_for_pixel(uint2(id.x, 2), 1);
    uint bounces = canonicalScene.cavity() ? 10u : 1u;
    InitialResult canonical = sampleInitialPath<LambertScene, 1>(
        canonicalScene, uint2(0), canonicalScene.primaryHit(), canonicalRng.state, bounces,
        settings, true, canonicalScene.useNEE(), canonicalScene.useMIS());
    InitialResult neighbor =
        sampleInitialPath(neighborScene, uint2(0), neighborScene.primaryHit(), neighborRng.state,
                          bounces, settings, true, neighborScene.useNEE(), neighborScene.useMIS());
    if (canonicalScene.mode == 9)
    {
        canonical = sampleInitialPath<LambertScene, 2>(
            canonicalScene, uint2(0), canonicalScene.primaryHit(), canonicalRng.state, bounces,
            settings, true, true, true);
        neighbor = sampleInitialPath<LambertScene, 2>(neighborScene, uint2(0),
                                                      neighborScene.primaryHit(), neighborRng.state,
                                                      bounces, settings, true, true, true);
    }
    neighbor.reservoir.M = 7.0;
    float3 initialEstimate =
        canonical.residual + canonical.reservoir.weight * canonical.reservoir.integrand;
    PrecomputedShiftData identity = shifted(canonicalScene, canonical.reservoir, settings);
    float3 error = abs(identity.pathThroughput - canonical.reservoir.integrand) /
                   max(float3(1.0), abs(canonical.reservoir.integrand));
    outputs[8 * id.x] = float4(initialEstimate, 1.0);
    outputs[8 * id.x + 1] = float4(error, identity.Jacobian);
    PrecomputedShiftData forward = shifted(canonicalScene, neighbor.reservoir, settings);
    PrecomputedShiftData reverse = shifted(neighborScene, canonical.reservoir, settings);
    PathReservoir temporal = temporal_resample(canonical.reservoir, neighbor.reservoir, reverse,
                                               forward, reuseRng.next2D());
    outputs[8 * id.x + 2] =
        float4(temporal.integrand * temporal.weight + canonical.residual, temporal.M);
    PairwiseFactor factor = { 8.0, 1.0 };
    PathReservoir spatial = empty_reservoir();
    float canonicalWeight = spatial_begin(factor);
    spatial_merge_neighbor(spatial, canonical.reservoir, neighbor.reservoir, forward, reverse,
                           factor, canonicalWeight, reuseRng.next1D());
    spatial_finish(spatial, canonical.reservoir, canonicalWeight, reuseRng.next1D());
    outputs[8 * id.x + 3] =
        float4(spatial.integrand * spatial.weight + canonical.residual, spatial.M);
    outputs[8 * id.x + 4] = float4(
        float(canonical.reservoir.weight > 0.0 &&
              canonical.reservoir.pathFlags.rcVertexLength() == MAXIMUM_RC_LENGTH),
        float(canonical.reservoir.weight > 0.0 && canonical.reservoir.pathFlags.lastVertexNEE() &&
              canonical.reservoir.pathFlags.rcVertexLength() ==
                  canonical.reservoir.pathFlags.pathLength() + 1),
        float(forward.Jacobian == 0.0), float(reverse.Jacobian == 0.0));
    outputs[8 * id.x + 5] =
        float4(forward.Jacobian, reverse.Jacobian, float(canonical.reservoir.weight > 0.0),
               float(neighbor.reservoir.weight > 0.0));
    outputs[8 * id.x + 6] = float4(
        canonical.residual + neighbor.reservoir.weight * forward.Jacobian * forward.pathThroughput,
        0.0);
    outputs[8 * id.x + 7] = float4(float(canonical.reservoir.pathFlags.pathLength() + 1u),
                                   float(neighbor.reservoir.pathFlags.pathLength() + 1u),
                                   float(canonical.reservoir.pathFlags.rcVertexLength()),
                                   float(neighbor.reservoir.pathFlags.rcVertexLength()));
}

/// 精确隔离逆向 footprint 拒绝；其余阈值与几何相同，临界条件为 1 / p_first = PI。
[numthreads(64, 1, 1)]
void restir_footprint_main(uint3 id: SV_DispatchThreadID)
{
    if (id.x >= 2u)
        return;
    LambertScene scene = testScene(0);
    scene.mode = 2;
    ShiftSettings settings = testSettings();
    settings.nearFieldDistanceThreshold = id.x == 0 ? 2.0 : 4.0;
    PathReservoir source = empty_reservoir();
    source.pathFlags.insertRcVertexLength(2u);
    source.pathFlags.insertPathLength(1u);
    source.pathFlags.insertLightType(LIGHT_EMISSIVE);
    source.reconnectionHit.data = uint4(2u, asuint(float3(0.0, 0.0, 2.0)));
    source.reconnectionIncidentRadiance = 1.0;
    source.reconnectionJacobian = 0.25;
    source.lightPdf = 0.0625;
    source.weight = 1.0;
    Surface previous =
        scene.loadSurfaceFromOrigin(scene.primaryHit(), scene.initialOrigin(0), false);
    float jacobian = 1.0;
    float3 value = shiftPathReconnection(scene, settings, 1.0, 1.0, jacobian, previous, source,
                                         true, true, false);
    outputs[id.x] = float4(value, jacobian);
}
