/// 解析 Lambert 场景 oracle；路径追踪、shift 与 GRIS 直接实例化生产泛型算法。
module restir_pt_test;

import restir_pt.types;
import restir_pt.rng;
import restir_pt.scene;
import restir_pt.initial;
import restir_pt.initial_vertex;
import restir_pt.shift;
import restir_pt.reservoir;
import restir_pt.resampling;
import geometry.ray;

using namespace restir_pt;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;
static const let PI = 3.14159265358979323846;

/// mode 2/3/10 为 z=2 无限发光平面，其 NEE 只采样边长4的子区域；10从双面灯背面接收光。
/// mode 7/8 为 z=0/1 的 Lambert 发光腔体，禁用NEE/MIS，最多10次散射。
/// variant 改变 primary 法线或横向位置，使双向 shift 的支持域/PDF 与源域不同。
public struct LambertScene : IRestirScene
{
    public var mode : uint;
    public var variant : uint;
    public var albedo : float3;
    public var radiance : float;
    public var extinction : float;
    public var angle : float;

    public func finiteLight()->bool
    {
        return mode == 2 || mode == 3 || mode == 10;
    }
    public func cavity()->bool
    {
        return mode == 7 || mode == 8;
    }
    public func useNEE()->bool
    {
        return mode != 1 && !cavity();
    }
    public func useMIS()->bool
    {
        return !cavity();
    }
    public func normal()->float3
    {
        let a : float = finiteLight() || cavity() || mode == 9 || variant == 0 ? 0.0 : angle;
        return float3(sin(a), 0.0, cos(a));
    }
    public func position()->float3
    {
        return finiteLight() || cavity() || mode == 6 ? float3(float(variant) * 0.7, 0.0, 0.0)
                                                      : float3(0.0);
    }
    public func primaryHit()->HitInfo
    {
        var hit : HitInfo;
        hit.data = uint4(mode == 5 ? 0u : 1u, asuint(position()));
        return hit;
    }
    public func loadSurface(hit: HitInfo, wo: float3, previous: bool)->Surface
    {
        var hit = hit;
        var result : Surface = {};
        result.hit = hit;
        result.point.position = asfloat(hit.data.yzw);
        result.point.normal =
            hit.data.x == 1 ? normal() : float3(0.0, 0.0, mode == 10 ? 1.0 : -1.0);
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
    public func loadSurfaceFromOrigin(hit: HitInfo, origin: float3, previous: bool)->Surface
    {
        var result : Surface = loadSurface(hit, float3(0.0), previous);
        let delta : float3 = origin - result.point.position;
        result.distance = length(delta);
        result.wo = delta / result.distance;
        result.frontFacing = dot(result.point.normal, result.wo) > 0.0;
        return result;
    }
    public func lightEndpoint(hit: HitInfo, previous: bool)->geometry::SurfacePoint
    {
        return loadSurface(hit, float3(0), previous).point;
    }
    public func emissionSideSupported(surface: Surface)->bool
    {
        return mode == 10 || surface.frontFacing;
    }
    public func shadeInitialVertex(surface: Surface, request: InitialRequest)->InitialVertex
    {
        return generic_initial_vertex(this, surface, request);
    }
    public func setIncident(inout surface: Surface, medium: float4)->void
    {
        surface.incident = medium;
    }
    public func trace(origin: float3, direction: float3, previous: bool, out hit: HitInfo,
                      out distance: float)
        ->bool
    {
        hit = {};
        distance = environmentDistance();
        if (cavity())
        {
            let upper : bool = direction.z > 0.0;
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
    public func visible(source: Surface, target: Surface, previous: bool)->bool
    {
        return true;
    }
    public func visibleDirection(source: Surface, direction: float3, previous: bool)->bool
    {
        return true;
    }
    public func transmittance(source: Surface, direction: float3, distance: float)->float3
    {
        return float3(exp(-extinction * distance));
    }
    public func environmentDistance()->float
    {
        return 2.0;
    }
    public func sampleBsdf(surface: Surface, u: float3)->BsdfSample
    {
        let n : float3 = surface.point.normal;
        let tangent = float3(n.z, 0.0, -n.x);
        let r : float = sqrt(u.x);
        let phi : float = 2.0 * PI * u.y;
        var result : BsdfSample = {};
        result.direction = tangent * (r * cos(phi)) + float3(0.0, r * sin(phi), 0.0) +
                           n * sqrt(max(0.0, 1.0 - u.x));
        result.weight = albedo;
        result.pdf = max(0.0, dot(n, result.direction)) / PI;
        result.componentPdf = result.pdf;
        result.medium = surface.incident;
        return result;
    }
    public func shadeInitial(surface: Surface, u: float3, direction: float3,
                             secondDirection: float3, sampleContinuation: bool)
        ->InitialShading
    {
        var sample : BsdfSample = {};
        if (sampleContinuation)
            sample = sampleBsdf(surface, u);
        return { sample, evaluateBsdf(surface, direction, -1), surface.emission,
                 evaluateBsdf(surface, secondDirection, -1) };
    }
    public func evaluateBsdf(surface: Surface, direction: float3, component: int)->BsdfEvaluation
    {
        let pdf : float = max(0.0, dot(surface.point.normal, direction)) / PI;
        return { albedo * pdf, pdf };
    }
    public func evaluateConnection(surface: Surface, direction: float3, distance: float,
                                   component: int)
        ->ConnectionEvaluation
    {
        return { evaluateBsdf(surface, direction, component),
                 transmittance(surface, direction, distance), surface.incident.x };
    }
    public func evaluateEmitter(surface: Surface, from: float3, previous: bool)->EmitterEvaluation
    {
        var surface = surface;
        surface = loadSurface(surface.hit, surface.wo, previous);
        var source : Surface = {};
        source.point.position = from;
        return { surface.emission,
                 lightPdf(source, surface.hit, -surface.wo, LIGHT_EMISSIVE, previous),
                 surface.point.position, surface.point.normal };
    }
    public func isSampleRough(surface: Surface, component: uint, pdf: float, threshold: float)->bool
    {
        return surface.roughness >= threshold;
    }
    public func sampleLight(surface: Surface, u: float3)->LightSample
    {
        var result : LightSample = {};
        result.radiance = float3(radiance);
        if (finiteLight())
        {
            let point = float3(4.0 * u.xy - 2.0, 2.0);
            let delta : float3 = point - surface.point.position;
            result.distance = length(delta);
            result.direction = delta / result.distance;
            result.hit.data = uint4(2u, asuint(point));
            result.pdf = result.distance * result.distance / (16.0 * result.direction.z);
            result.lightType = LIGHT_EMISSIVE;
        }
        else
        {
            let cosine : float = u.x;
            let sine : float = sqrt(1.0 - cosine * cosine);
            var phi : float = 2.0 * PI * u.y;
            if (mode == 9)
                phi = PI * (u.y + float(u.z >= 0.5));
            let n : float3 = surface.point.normal;
            result.direction = float3(n.z, 0.0, -n.x) * (sine * cos(phi)) +
                               float3(0.0, sine * sin(phi), 0.0) + n * cosine;
            result.distance = environmentDistance();
            result.pdf = (mode == 9 ? 1.0 : 0.5) / PI;
            result.lightType = LIGHT_ENV;
            result.radiance = environment(result.direction, surface.point.position, false);
        }
        return result;
    }
    public func lightPdf(surface: Surface, light: HitInfo, direction: float3, lightType: uint,
                         previous: bool)
        ->float
    {
        if (cavity())
            return 0.0;
        if (!finiteLight())
            return dot(surface.point.normal, direction) > 0.0 ? (mode == 9 ? 1.0 : 0.5) / PI : 0.0;
        let point : float3 = asfloat(light.data.yzw);
        if (any(abs(point.xy) > 2.0) || direction.z <= 0.0)
            return 0.0;
        let delta : float3 = point - surface.point.position;
        return dot(delta, delta) / (16.0 * direction.z);
    }
    public func environment(direction: float3, origin: float3, previous: bool)->float3
    {
        if (cavity())
            return 0.0;
        return float3(radiance * (mode == 6 ? 1.0 + 0.5 * origin.x : 1.0));
    }
    public func isPointLight(hit: HitInfo)->bool
    {
        return false;
    }
    public func initialOrigin(pixel: uint2)->float3
    {
        return position() + normal() * (cavity() ? 0.5 : 1.0);
    }
    public func initialDirection(pixel: uint2)->float3
    {
        return -normal();
    }
    public func initialMedium(pixel: uint2)->float4
    {
        return float4(1.0, extinction, extinction, extinction);
    }
    public func primaryRayFootprint(surface: Surface, pixel: uint2, previous: bool)->float
    {
        return 1.0;
    }
}

func testScene(variant: uint)->LambertScene
{
    var scene : LambertScene;
    scene.mode = inputs[0].x;
    scene.variant = variant;
    let a : float4 = asfloat(inputs[1]);
    scene.extinction = a.z;
    scene.angle = a.w;
    let b : float4 = asfloat(inputs[2]);
    scene.albedo = b.xyz;
    scene.radiance = b.w;
    return scene;
}

func testSettings()->ShiftSettings
{
    var settings : ShiftSettings = {};
    settings.nearFieldDistanceSigma = 0.0;
    settings.specularRoughnessSigma = 0.0;
    settings.specularRoughnessThreshold = asfloat(inputs[1].x);
    settings.nearFieldDistanceThreshold = asfloat(inputs[1].y);
    return settings;
}

func shifted(inout destination: LambertScene, source: PathReservoir, settings: ShiftSettings)
    ->PrecomputedShiftData
{
    var source = source;
    let primary : Surface = destination.loadSurfaceFromOrigin(destination.primaryHit(),
                                                              destination.initialOrigin(0), false);
    var result : PrecomputedShiftData = {};
    result.pathThroughput =
        shiftPath(destination, primary, source, 1.0, settings, true, destination.useNEE(),
                  destination.useMIS(), false, result.Jacobian);
    result.reconnectionJacobian = source.reconnectionJacobian;
    return result;
}

/// 每线程一个独立 ensemble；输出初采、恒等移位误差、temporal、spatial 与分支计数。
[numthreads(64, 1, 1)]
func restir_mean_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= inputs[0].y)
        return;
    var canonicalScene : LambertScene = testScene(0);
    var neighborScene : LambertScene = testScene(1);
    let settings : ShiftSettings = testSettings();
    let canonicalRng : TinyUniformSampleGenerator = rng_for_pixel(uint2(id.x, 0), 1);
    let neighborRng : TinyUniformSampleGenerator = rng_for_pixel(uint2(id.x, 1), 1);
    var reuseRng : TinyUniformSampleGenerator = rng_for_pixel(uint2(id.x, 2), 1);
    let bounces : uint = canonicalScene.cavity() ? 10u : 1u;
    var canonical : InitialResult = sampleInitialPath<LambertScene, 1>(
                        canonicalScene, uint2(0), canonicalScene.primaryHit(), canonicalRng.state,
                        bounces, settings, true, canonicalScene.useNEE(), canonicalScene.useMIS());
    var neighbor : InitialResult = sampleInitialPath(
                       neighborScene, uint2(0), neighborScene.primaryHit(), neighborRng.state,
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
    let initialEstimate
        : float3 = canonical.residual + canonical.reservoir.weight * canonical.reservoir.integrand;
    let identity : PrecomputedShiftData = shifted(canonicalScene, canonical.reservoir, settings);
    let error : float3 = abs(identity.pathThroughput - canonical.reservoir.integrand) /
                         max(float3(1.0), abs(canonical.reservoir.integrand));
    outputs[8 * id.x] = float4(initialEstimate, 1.0);
    outputs[8 * id.x + 1] = float4(error, identity.Jacobian);
    let forward : PrecomputedShiftData = shifted(canonicalScene, neighbor.reservoir, settings);
    let reverse : PrecomputedShiftData = shifted(neighborScene, canonical.reservoir, settings);
    let temporal : PathReservoir = temporal_resample(canonical.reservoir, neighbor.reservoir,
                                                     reverse, forward, reuseRng.next2D());
    outputs[8 * id.x + 2] =
        float4(temporal.integrand * temporal.weight + canonical.residual, temporal.M);
    let factor : PairwiseFactor = { 8.0, 1.0 };
    var spatial : PathReservoir = empty_reservoir();
    var canonicalWeight : float = spatial_begin(factor);
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
func restir_footprint_main(uint3 id: SV_DispatchThreadID)->void
{
    if (id.x >= 2u)
        return;
    var scene : LambertScene = testScene(0);
    scene.mode = 2;
    var settings : ShiftSettings = testSettings();
    settings.nearFieldDistanceThreshold = id.x == 0 ? 2.0 : 4.0;
    var source : PathReservoir = empty_reservoir();
    source.pathFlags.insertRcVertexLength(2u);
    source.pathFlags.insertPathLength(1u);
    source.pathFlags.insertLightType(LIGHT_EMISSIVE);
    source.reconnectionHit.data = uint4(2u, asuint(float3(0.0, 0.0, 2.0)));
    source.reconnectionIncidentRadiance = 1.0;
    source.reconnectionJacobian = 0.25;
    source.lightPdf = 0.0625;
    source.weight = 1.0;
    let previous
        : Surface = scene.loadSurfaceFromOrigin(scene.primaryHit(), scene.initialOrigin(0), false);
    var jacobian = 1.0;
    let value : float3 = shiftPathReconnection(scene, settings, 1.0, 1.0, jacobian, previous,
                                               source, true, true, false);
    outputs[id.x] = float4(value, jacobian);
}
