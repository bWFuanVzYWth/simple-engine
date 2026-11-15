#include "compute.slang"
import atmosphere.terrain_frame;
#include "shadow_profile.inc.slang"
[[vk::binding(8, 0)]]
var frames : StructuredBuffer<TerrainFrame>;
[[vk::binding(9, 0)]]
var shadowCells : RWStructuredBuffer<uint4>;
[[vk::binding(10, 0)]]
var previousProfiles : RWStructuredBuffer<uint>;
[[vk::binding(11, 0)]]
[vk::image_format("rgba16f")]
var terrain_light : RWTexture3D<float4>;
[[vk::binding(12, 0)]]
var first_hits : StructuredBuffer<uint>;
#include "shadow_source.inc.slang"
groupshared var profileChanged : uint;
groupshared var sameView : uint;
func updateShadowProfile(slice: uint, lane: uint)->bool
{
    var start : uint = slice * (SUN_SHADOW_PROFILE_TOTAL_CAPACITY + SUN_SHADOW_CASCADE_COUNT + 1u);
    if (lane == 0)
    {
        sameView = !terrain_full(frames[0]) && previousProfiles[start] == frames[0].source.state.z;
        previousProfiles[start] = frames[0].source.state.z;
        profileChanged = sameView == 0 ? 1u : 0u;
    }
    GroupMemoryBarrierWithGroupSync();
    let reuse : bool = sameView != 0;
    start += 1;
    var changed : bool = (first_hits[slice * TERRAIN_RADIAL + lane] >> 31u) != 0;
    if (lane < SUN_SHADOW_CASCADE_COUNT)
    {
        let count : uint = asuint(sunShadowProfileCounts[lane]);
        if (reuse)
            changed = changed || previousProfiles[start + lane] != count;
        previousProfiles[start + lane] = count;
    }
    for (var cascade = 0; cascade < SUN_SHADOW_CASCADE_COUNT; cascade++)
    {
        let base : int = sunShadowProfileBase(cascade);
        for (var i = int(lane); i < sunShadowProfileCounts[cascade]; i += 128)
        {
            let index : uint = start + SUN_SHADOW_CASCADE_COUNT + base + i;
            let depth : uint = asuint(sunShadowProfileEntries[base + i].y);
            if (reuse)
                changed = changed || previousProfiles[index] != depth;
            previousProfiles[index] = depth;
        }
    }
    if (changed)
        InterlockedOr(profileChanged, 1u);
    GroupMemoryBarrierWithGroupSync();
    return profileChanged != 0;
}

func terrain_rgb(spectrum: float4, scale: float3)->float3
{
    var rgb : float3 = 0;
    for (var k: uint = 0; k < 4; ++k)
        rgb += spectrum[k] * p.rgb[k].xyz;
    return max(rgb * scale, 0);
}

// 各深度段沿相机可见的空气积分受光源项。末层单独保存天空需扣除的遮挡量及远场边界可见度。
[shader("compute")]
[numthreads(128, 1, 1)]
func terrain_integrate(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void
{
    let slice : uint = terrain_slice_index(frames[0], group.y);
    terrain_slice<false>(lane, slice);
    if (!updateShadowProfile(slice, lane))
        return;
    let f : TerrainFrame = frames[0];
    let ray : float3 = terrain_ray(f, lane, slice);
    let sun : float3 = f.source.sun.xyz;
    let height : float = f.source.camera.w;
    let radius : float = p.sun.w + height;
    let first_hit : float = asfloat(first_hits[slice * TERRAIN_RADIAL + lane] & 0x7fffffffu);
    var entry : float = 0;
    var end : float = first_hit * 0.001;
    var entry_height : float = height;
    var entry_mu : float = ray.y;
    if (height > p.medium.x)
    {
        let b : float = radius * ray.y;
        let c : float = (height - p.medium.x) * (radius + p.sun.w + p.medium.x);
        let disc : float = b * b - c;
        if (b < 0 && disc > 0)
        {
            let root : float = sqrt(disc);
            entry = c / (-b + root);
            entry_height = p.medium.x;
            entry_mu = -root / (p.sun.w + entry_height);
        }
        else
            entry = end;
    }
    if (entry < end)
        end = min(end,
                  entry + boundary(p, entry_height, entry_mu, ground(p, entry_height, entry_mu)));
    let nu = clamp(dot(ray, sun), -1.0, 1.0);
    let stencil = phase_stencil(nu);
    var phase : float4[5];
    phase[0] = float4(3.0 * (1.0 + nu * nu) / (16.0 * atmosphere::PI));
    for (var k: uint = 0; k < 4; ++k)
        phase[k + 1] = tabulated_phase(stencil, k);
    var lit : float4 = 0;
    var blocked : float4 = 0;
    var transmission : float4 = 1;
    var previous : float = 0;
    var boundary_visibility : float = 1;
    let projected = float2(dot(ray, sunShadowProfileBasisU), dot(ray, sunShadowProfileBasisV));
    let rate : float = dot(projected, sunShadowProfileDirection);
    let covered_metres
        : float =
              rate > SUN_SHADOW_PROFILE_EPSILON
                  ? min(TERRAIN_RANGE_M, sunShadowProfileCascadeEnd(shadowSource<false>(),
                                                                    SUN_SHADOW_CASCADE_COUNT - 1) /
                                             rate)
                  : TERRAIN_RANGE_M;
    for (var z: uint = 0; z < TERRAIN_DEPTH; ++z)
    {
        let t = float(z + 1u) / float(TERRAIN_DEPTH);
        let edge : float = min(end, TERRAIN_RANGE_M * 0.001 * t * t);
        let start : float = max(previous, entry);
        if (edge > start)
        {
            let dt : float = (edge - start) * 0.5;
            for (var step: uint = 0; step < 2; ++step)
            {
                let a : float = start + float(step) * dt;
                let d : float = a + dt * 0.5;
                let h : float = height_at(p, height, ray.y, d);
                let up : float3 = (float3(0, radius, 0) + ray * d) / (p.sun.w + h);
                let c : Medium = medium(h);
                let vis : float = localSunVisibilityInterval(shadowSource<false>(), ray, a * 1000.0,
                                                             (a + dt) * 1000.0);
                // 有限表面只需要受光积分。只有可达近场末端的射线还要给天空提供遮挡积分。
                if (vis > 0 || first_hit >= TERRAIN_RANGE_M)
                {
                    var direct : float4 = 0;
                    for (var k: uint = 0; k < 5; ++k)
                        direct += c.scattering[k] * phase[k];
                    let mus : float = dot(up, sun);
                    var source : float4 = direct *
                                          sun_transmittance(p, tau_read, linear_sampler, h, mus) *
                                          p.solar;
                    if (p.solve.w != 1u)
                        source += indirect(h, dot(up, ray), mus, nu, phase_coord(nu), c);
                    let energy : float4 =
                                     transmission * source * segment_integral(c.extinction, dt);
                    lit += energy * vis;
                    blocked += energy * (1. - vis);
                }
                transmission *= exp(-dt * c.extinction);
                let measured : float = min((a + dt) * 1000.0, covered_metres) - a * 1000.0;
                if (measured > 0)
                    boundary_visibility = saturate(vis * dt * 1000.0 / measured);
            }
        }
        previous = edge;
        terrain_light[uint3(lane, slice, z)] = float4(terrain_rgb(lit, f.source.scale.xyz), 0);
    }
    terrain_light[uint3(lane, slice, TERRAIN_DEPTH)] =
        float4(terrain_rgb(blocked, f.source.scale.xyz), boundary_visibility);
}
