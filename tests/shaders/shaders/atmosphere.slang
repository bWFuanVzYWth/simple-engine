// 来源：Sky Tracer 9e9430d；测试生产共享函数的球形边界与常系数传输恒等式。
import atmosphere.parameters;
import atmosphere.sampling;
import atmosphere.terrain_frame;
using namespace atmosphere;
#include "atmosphere/shadow_profile.inc.slang"

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

[shader("compute")]
[numthreads(64, 1, 1)]
func atmosphere_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : float4 = asfloat(inputs[2 * id.x]);
    let b : uint4 = inputs[2 * id.x + 1];
    if (b.y == 0)
        outputs[id.x] = asuint(segment_transport(a, asfloat(b.x)).integral);
    else if (b.y == 2)
        outputs[id.x] = asuint(segment_transport(a, asfloat(b.x)).transmittance);
    else {
        var p : Params = (Params)0;
        p.planet_radius_km = 6360.0;
        p.atmosphere_height_km = 120.0;
        let hit : bool = ground(p, a.x, a.y);
        let d : float = boundary(p, a.x, a.y, hit);
        outputs[id.x] = uint4(asuint(float3(d, height_at(p, a.x, a.y, d), float(hit))), id.x + 1);
    }
}

struct IntervalProfileSource : IShadowProfileSource {
    func texel_size(cascade: int)->float {
        return shadow_texel_size(cascade);
    }
    func blocker_depth(cascade: int, node: int2)->float {
        return 0;
    }
};

[shader("compute")]
[numthreads(64, 1, 1)]
func epipolar_plane_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let sun = asfloat(inputs[2 * id.x]).xyz;
    let at = inputs[2 * id.x + 1];
    var input = TerrainInput();
    input.forward = float3(0, 0, 1);
    input.right = float3(1.78, 0, 0);
    input.up = float3(0, 1, 0);
    input.sun_direction = sun;
    let frame = terrain_frame(input);
    let ray = terrain_ray(frame, at.x, at.y);
    let edge = terrain_ray(frame, TERRAIN_RADIAL - 1, at.y);
    outputs[id.x] = asuint(float4(ray, dot(ray, normalize(cross(sun, edge)))));
}
[shader("compute")]
[numthreads(1, 1, 1)]
func shadow_interval_main(uint3 id: SV_DispatchThreadID)->void {
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a = asfloat(inputs[2 * id.x]);
    let b = asfloat(inputs[2 * id.x + 1]);
    initialize_shadow_profile(float3(0, 0, 1), float2(0), a.w, float3(1, 0, 0), float3(1, 0, 0),
                              float3(0, 1, 0));
    shadow_profile_counts[0] = 3;
    shadow_profile_entries[0] = float2(63.5, -0.25);
    shadow_profile_entries[1] = float2(95.25, 0.5);
    shadow_profile_entries[2] = float2(127.0, 1.25);
    for (var cascade = 1; cascade < SUN_SHADOW_CASCADE_COUNT; ++cascade) {
        shadow_profile_counts[cascade] = 1;
        shadow_profile_entries[shadow_profile_base(cascade)] =
            float2(shadow_profile_cascade_end(IntervalProfileSource(), cascade), 0.35);
    }
    let visibility =
        shadow_visible_interval(IntervalProfileSource(), shadow_profile_ray(a.xyz), b.x, b.y);
    outputs[id.x] = uint4(asuint(visibility), 0, 0, id.x + 1);
}
