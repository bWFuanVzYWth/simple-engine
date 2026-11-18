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
public struct LambertScene : IRestirScene {
    public var mode : uint;
    public var variant : uint;
    public var albedo : float3;
    public var radiance : float;
    public var extinction : float;
    public var angle : float;

    public func finiteLight()->bool {
        return mode == 2 || mode == 3 || mode == 10;
    }
    public func cavity()->bool {
        return mode == 7 || mode == 8;
    }
    public func use_nee()->bool {
        return mode != 1 && !cavity();
    }
    public func use_mis()->bool {
        return !cavity();
    }
    public func normal()->float3 {
        let a : float = finiteLight() || cavity() || mode == 9 || variant == 0 ? 0.0 : angle;
        return float3(sin(a), 0.0, cos(a));
    }
    public func position()->float3 {
        return finiteLight() || cavity() || mode == 6 ? float3(float(variant) * 0.7, 0.0, 0.0)
                                                      : float3(0.0);
    }
    public func primary_hit()->HitInfo {
        var hit : HitInfo;
        hit.data = uint4(mode == 5 ? 0u : 1u, asuint(position()));
        return hit;
    }
    public func load_surface(hit: HitInfo, wo: float3, previous: bool)->Surface {
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
        result.support_simple_roughness_threshold = true;
        result.has_non_delta_lobes = cavity() || hit.data.x == 1;
        result.front_facing = dot(result.point.normal, wo) > 0.0;
        return result;
    }
    public func load_surface_from_origin(hit: HitInfo, origin: float3, previous: bool)->Surface {
        var result : Surface = load_surface(hit, float3(0.0), previous);
        let delta : float3 = origin - result.point.position;
        result.distance = length(delta);
        result.wo = delta / result.distance;
        result.front_facing = dot(result.point.normal, result.wo) > 0.0;
        return result;
    }
    public func light_endpoint(hit: HitInfo, previous: bool)->geometry::SurfacePoint {
        return load_surface(hit, float3(0), previous).point;
    }
    public func emission_side_supported(surface: Surface)->bool {
        return mode == 10 || surface.front_facing;
    }
    public func shade_initial_vertex(surface: Surface, request: InitialRequest)->InitialVertex {
        return generic_initial_vertex(this, surface, request);
    }
    public func set_incident(inout surface: Surface, medium: float4)->void {
        surface.incident = medium;
    }
    public func trace(origin: float3, direction: float3, previous: bool, out hit: HitInfo,
                      out distance: float)
        ->bool {
        hit = {};
        distance = environment_distance();
        if (cavity()) {
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
    public func visible(source: Surface, target: Surface, previous: bool)->bool {
        return true;
    }
    public func visible_direction(source: Surface, direction: float3, previous: bool)->bool {
        return true;
    }
    public func transmittance(source: Surface, direction: float3, distance: float)->float3 {
        return float3(exp(-extinction * distance));
    }
    public func environment_distance()->float {
        return 2.0;
    }
    public func sample_bsdf(surface: Surface, u: float3)->BsdfSample {
        let n : float3 = surface.point.normal;
        let tangent = float3(n.z, 0.0, -n.x);
        let r : float = sqrt(u.x);
        let phi : float = 2.0 * PI * u.y;
        var result : BsdfSample = {};
        result.direction = tangent * (r * cos(phi)) + float3(0.0, r * sin(phi), 0.0) +
                           n * sqrt(max(0.0, 1.0 - u.x));
        result.weight = albedo;
        result.pdf = max(0.0, dot(n, result.direction)) / PI;
        result.component_pdf = result.pdf;
        result.medium = surface.incident;
        return result;
    }
    public func shade_initial(surface: Surface, u: float3, direction: float3,
                              secondDirection: float3, sample_continuation: bool)
        ->InitialShading {
        var sample : BsdfSample = {};
        if (sample_continuation)
            sample = sample_bsdf(surface, u);
        return { sample, evaluate_bsdf(surface, direction, -1), surface.emission,
                 evaluate_bsdf(surface, secondDirection, -1) };
    }
    public func evaluate_bsdf(surface: Surface, direction: float3, component: int)->BsdfEvaluation {
        let pdf : float = max(0.0, dot(surface.point.normal, direction)) / PI;
        return { albedo * pdf, pdf };
    }
    public func evaluate_connection(surface: Surface, direction: float3, distance: float,
                                    component: int)
        ->ConnectionEvaluation {
        return { evaluate_bsdf(surface, direction, component),
                 transmittance(surface, direction, distance), surface.incident.x };
    }
    public func evaluate_emitter(surface: Surface, from: float3, previous: bool)
        ->EmitterEvaluation {
        var surface = surface;
        surface = load_surface(surface.hit, surface.wo, previous);
        var source : Surface = {};
        source.point.position = from;
        return { surface.emission,
                 light_pdf(source, surface.hit, -surface.wo, LIGHT_EMISSIVE, previous),
                 surface.point.position, surface.point.normal };
    }
    public func is_sample_rough(surface: Surface, component: uint, pdf: float, threshold: float)
        ->bool {
        return surface.roughness >= threshold;
    }
    public func sample_light(surface: Surface, u: float3)->LightSample {
        var result : LightSample = {};
        result.radiance = float3(radiance);
        if (finiteLight()) {
            let point = float3(4.0 * u.xy - 2.0, 2.0);
            let delta : float3 = point - surface.point.position;
            result.distance = length(delta);
            result.direction = delta / result.distance;
            result.hit.data = uint4(2u, asuint(point));
            result.pdf = result.distance * result.distance / (16.0 * result.direction.z);
            result.light_type = LIGHT_EMISSIVE;
        } else {
            let cosine : float = u.x;
            let sine : float = sqrt(1.0 - cosine * cosine);
            var phi : float = 2.0 * PI * u.y;
            if (mode == 9)
                phi = PI * (u.y + float(u.z >= 0.5));
            let n : float3 = surface.point.normal;
            result.direction = float3(n.z, 0.0, -n.x) * (sine * cos(phi)) +
                               float3(0.0, sine * sin(phi), 0.0) + n * cosine;
            result.distance = environment_distance();
            result.pdf = (mode == 9 ? 1.0 : 0.5) / PI;
            result.light_type = LIGHT_ENV;
            result.radiance = environment(result.direction, surface.point.position, false);
        }
        return result;
    }
    public func light_pdf(surface: Surface, light: HitInfo, direction: float3, light_type: uint,
                          previous: bool)
        ->float {
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
    public func environment(direction: float3, origin: float3, previous: bool)->float3 {
        if (cavity())
            return 0.0;
        return float3(radiance * (mode == 6 ? 1.0 + 0.5 * origin.x : 1.0));
    }
    public func is_point_light(hit: HitInfo)->bool {
        return false;
    }
    public func initial_origin(pixel: uint2)->float3 {
        return position() + normal() * (cavity() ? 0.5 : 1.0);
    }
    public func initial_direction(pixel: uint2)->float3 {
        return -normal();
    }
    public func initial_medium(pixel: uint2)->float4 {
        return float4(1.0, extinction, extinction, extinction);
    }
    public func primary_ray_footprint(surface: Surface, pixel: uint2, previous: bool)->float {
        return 1.0;
    }
}

func testScene(variant: uint)->LambertScene {
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

func testSettings()->ShiftSettings {
    var settings : ShiftSettings = {};
    settings.near_field_distance_sigma = 0.0;
    settings.specular_roughness_sigma = 0.0;
    settings.specular_roughness_threshold = asfloat(inputs[1].x);
    settings.near_field_distance_threshold = asfloat(inputs[1].y);
    return settings;
}

func shifted(inout destination: LambertScene, source: PathReservoir, settings: ShiftSettings)
    ->PrecomputedShiftData {
    var source = source;
    let primary : Surface = destination.load_surface_from_origin(
                      destination.primary_hit(), destination.initial_origin(0), false);
    var result : PrecomputedShiftData = {};
    result.path_throughput =
        shift_path(destination, primary, source, 1.0, settings, true, destination.use_nee(),
                   destination.use_mis(), false, result.jacobian);
    result.reconnection_jacobian = source.reconnection_jacobian;
    return result;
}

/// 每线程一个独立 ensemble；输出初采、恒等移位误差、temporal、spatial 与分支计数。
[numthreads(64, 1, 1)]
func restir_mean_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x >= inputs[0].y)
        return;
    var canonicalScene : LambertScene = testScene(0);
    var neighborScene : LambertScene = testScene(1);
    let settings : ShiftSettings = testSettings();
    let canonicalRng : TinyUniformSampleGenerator = rng_for_pixel(uint2(id.x, 0), 1);
    let neighborRng : TinyUniformSampleGenerator = rng_for_pixel(uint2(id.x, 1), 1);
    var reuseRng : TinyUniformSampleGenerator = rng_for_pixel(uint2(id.x, 2), 1);
    let bounces : uint = canonicalScene.cavity() ? 10u : 1u;
    var canonical
        : InitialResult = sample_initial_path<LambertScene, 1>(
              canonicalScene, uint2(0), canonicalScene.primary_hit(), canonicalRng.state, bounces,
              settings, true, canonicalScene.use_nee(), canonicalScene.use_mis());
    var neighbor : InitialResult = sample_initial_path(
                       neighborScene, uint2(0), neighborScene.primary_hit(), neighborRng.state,
                       bounces, settings, true, neighborScene.use_nee(), neighborScene.use_mis());
    if (canonicalScene.mode == 9) {
        canonical = sample_initial_path<LambertScene, 2>(
            canonicalScene, uint2(0), canonicalScene.primary_hit(), canonicalRng.state, bounces,
            settings, true, true, true);
        neighbor = sample_initial_path<LambertScene, 2>(
            neighborScene, uint2(0), neighborScene.primary_hit(), neighborRng.state, bounces,
            settings, true, true, true);
    }
    neighbor.reservoir.sample_mass = 7.0;
    let initialEstimate
        : float3 = canonical.residual + canonical.reservoir.weight * canonical.reservoir.integrand;
    let identity : PrecomputedShiftData = shifted(canonicalScene, canonical.reservoir, settings);
    let error : float3 = abs(identity.path_throughput - canonical.reservoir.integrand) /
                         max(float3(1.0), abs(canonical.reservoir.integrand));
    outputs[8 * id.x] = float4(initialEstimate, 1.0);
    outputs[8 * id.x + 1] = float4(error, identity.jacobian);
    let forward : PrecomputedShiftData = shifted(canonicalScene, neighbor.reservoir, settings);
    let reverse : PrecomputedShiftData = shifted(neighborScene, canonical.reservoir, settings);
    let temporal : PathReservoir = temporal_resample(canonical.reservoir, neighbor.reservoir,
                                                     reverse, forward, reuseRng.next_2d());
    outputs[8 * id.x + 2] =
        float4(temporal.integrand * temporal.weight + canonical.residual, temporal.sample_mass);
    let factor : PairwiseFactor = { 8.0, 1.0 };
    var spatial : PathReservoir = empty_reservoir();
    var canonical_weight : float = spatial_begin(factor);
    spatial_merge_neighbor(spatial, canonical.reservoir, neighbor.reservoir, forward, reverse,
                           factor, canonical_weight, reuseRng.next_1d());
    spatial_finish(spatial, canonical.reservoir, canonical_weight, reuseRng.next_1d());
    outputs[8 * id.x + 3] =
        float4(spatial.integrand * spatial.weight + canonical.residual, spatial.sample_mass);
    outputs[8 * id.x + 4] =
        float4(float(canonical.reservoir.weight > 0.0 &&
                     canonical.reservoir.path_flags.rc_vertex_length() == MAXIMUM_RC_LENGTH),
               float(canonical.reservoir.weight > 0.0 &&
                     canonical.reservoir.path_flags.last_vertex_nee() &&
                     canonical.reservoir.path_flags.rc_vertex_length() ==
                         canonical.reservoir.path_flags.path_length() + 1),
               float(forward.jacobian == 0.0), float(reverse.jacobian == 0.0));
    outputs[8 * id.x + 5] =
        float4(forward.jacobian, reverse.jacobian, float(canonical.reservoir.weight > 0.0),
               float(neighbor.reservoir.weight > 0.0));
    outputs[8 * id.x + 6] = float4(
        canonical.residual + neighbor.reservoir.weight * forward.jacobian * forward.path_throughput,
        0.0);
    outputs[8 * id.x + 7] = float4(float(canonical.reservoir.path_flags.path_length() + 1u),
                                   float(neighbor.reservoir.path_flags.path_length() + 1u),
                                   float(canonical.reservoir.path_flags.rc_vertex_length()),
                                   float(neighbor.reservoir.path_flags.rc_vertex_length()));
}

/// 精确隔离逆向 footprint 拒绝；其余阈值与几何相同，临界条件为 1 / p_first = PI。
[numthreads(64, 1, 1)]
func restir_footprint_main(uint3 id: SV_DispatchThreadID)->void {
    if (id.x >= 2u)
        return;
    var scene : LambertScene = testScene(0);
    scene.mode = 2;
    var settings : ShiftSettings = testSettings();
    settings.near_field_distance_threshold = id.x == 0 ? 2.0 : 4.0;
    var source : PathReservoir = empty_reservoir();
    source.path_flags.insert_rc_vertex_length(2u);
    source.path_flags.insert_path_length(1u);
    source.path_flags.insert_light_type(LIGHT_EMISSIVE);
    source.reconnection_hit.data = uint4(2u, asuint(float3(0.0, 0.0, 2.0)));
    source.reconnection_incident_radiance = 1.0;
    source.reconnection_jacobian = 0.25;
    source.light_pdf = 0.0625;
    source.weight = 1.0;
    let previous : Surface = scene.load_surface_from_origin(scene.primary_hit(),
                                                            scene.initial_origin(0), false);
    var jacobian = 1.0;
    let value : float3 = shift_path_reconnection(scene, settings, 1.0, 1.0, jacobian, previous,
                                                 source, true, true, false);
    outputs[id.x] = float4(value, jacobian);
}
