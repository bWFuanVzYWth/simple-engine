import math.constants;
#include "compute.slang"
import atmosphere.terrain_frame;
#include "shadow_profile.inc.slang"
[[vk::binding(8, 0)]]
var frames : StructuredBuffer<TerrainFrame>;
[[vk::binding(9, 0)]]
var shadow_cells : RWStructuredBuffer<uint4>;
[[vk::binding(10, 0)]]
var previous_profiles : RWStructuredBuffer<uint>;
[[vk::binding(11, 0)]]
[vk::image_format("rgba16f")]
var terrain_light : RWTexture3D<float4>;
[[vk::binding(12, 0)]]
var first_hits : StructuredBuffer<uint>;
#include "shadow_source.inc.slang"
groupshared var profile_changed : uint;
groupshared var same_view : uint;
func update_shadow_profile(slice: uint, lane: uint)->bool
{
    var start : uint = slice * (SUN_SHADOW_PROFILE_TOTAL_CAPACITY + SUN_SHADOW_CASCADE_COUNT + 1u);
    if (lane == 0)
    {
        same_view =
            !terrain_full(frames[0]) && previous_profiles[start] == frames[0].source.view_version;
        previous_profiles[start] = frames[0].source.view_version;
        profile_changed = same_view == 0 ? 1u : 0u;
    }
    GroupMemoryBarrierWithGroupSync();
    let reuse : bool = same_view != 0;
    start += 1;
    var changed : bool = (first_hits[slice * TERRAIN_RADIAL + lane] >> 31u) != 0;
    if (lane < SUN_SHADOW_CASCADE_COUNT)
    {
        let count : uint = asuint(shadow_profile_counts[lane]);
        if (reuse)
            changed = changed || previous_profiles[start + lane] != count;
        previous_profiles[start + lane] = count;
    }
    for (var cascade = 0; cascade < SUN_SHADOW_CASCADE_COUNT; cascade++)
    {
        let base = shadow_profile_base(cascade);
        for (var i = int(lane); i < shadow_profile_counts[cascade]; i += 128)
        {
            let index : uint = start + SUN_SHADOW_CASCADE_COUNT + base + i;
            let depth : uint = asuint(shadow_profile_entries[base + i].y);
            if (reuse)
                changed = changed || previous_profiles[index] != depth;
            previous_profiles[index] = depth;
        }
    }
    if (changed)
        InterlockedOr(profile_changed, 1u);
    GroupMemoryBarrierWithGroupSync();
    return profile_changed != 0;
}

func terrain_rgb(spectrum: float4, scale: float3)->float3
{
    var rgb : float3 = 0;
    for (var k: uint = 0; k < 4; ++k)
        rgb += spectrum[k] * p.spectral_to_rec2020[k].xyz;
    return max(rgb * scale, 0);
}

// 各深度段沿相机可见的空气积分受光源项。末层单独保存天空需扣除的遮挡量及远场边界可见度。
[shader("compute")]
[numthreads(128, 1, 1)]
func terrain_integrate(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)->void
{
    let slice = terrain_slice_index(frames[0], group.y);
    terrain_slice<false>(lane, slice);
    if (!update_shadow_profile(slice, lane))
        return;
    let f : TerrainFrame = frames[0];
    let segment
        : EpipolarSegment = { terrain_segment_endpoints.xy, terrain_segment_endpoints.zw, false };
    let ray = terrain_ray(f, lane, segment);
    let sun = f.source.sun_direction;
    let height = f.source.observer_height_km;
    let radius = p.planet_radius_km + height;
    let radial = radial_ray(p, height, ray.y);
    let radial_dot = radial.twice_radial_dot * 0.5;
    let sun_radial_dot = radius * sun.y;
    let ray_squared = dot(ray, ray);
    let first_hit : float = asfloat(first_hits[slice * TERRAIN_RADIAL + lane] & 0x7fffffffu);
    var entry : float = 0;
    var end : float = first_hit * 0.001;
    var entry_height : float = height;
    var entry_mu : float = ray.y;
    if (height > p.atmosphere_height_km)
    {
        let b = radius * ray.y;
        let c = (height - p.atmosphere_height_km) *
                (radius + p.planet_radius_km + p.atmosphere_height_km);
        let disc = b * b - c;
        if (b < 0 && disc > 0)
        {
            let root = sqrt(disc);
            entry = c / (-b + root);
            entry_height = p.atmosphere_height_km;
            entry_mu = -root / (p.planet_radius_km + entry_height);
        }
        else
            entry = end;
    }
    if (entry < end)
        end = min(end,
                  entry + boundary(p, entry_height, entry_mu, ground(p, entry_height, entry_mu)));
    let ray_sun_dot = dot(ray, sun);
    let nu = clamp(ray_sun_dot, -1.0, 1.0);
    let power = phase_power(nu);
    let stencil = phase_interval_from_power(power);
    let angle : IndirectRayGeometry = { nu, 1.0 - nu * nu, phase_coord(nu, power) };
    var phase : float4[5];
    phase[0] = float4((3.0 / (16.0 * math::pi)) * (1.0 + nu * nu));
    for (var k: uint = 0; k < 4; ++k)
        phase[k + 1] = tabulated_phase(stencil, k);
    var lit : float4 = 0;
    var blocked : float4 = 0;
    var transmission : float4 = 1;
    var previous : float = 0;
    var boundary_visibility : float = 1;
    let shadow_ray = shadow_profile_ray(ray);
    let rate = shadow_ray.scalar_rate;
    let covered_metres
        : float =
              rate > SUN_SHADOW_PROFILE_EPSILON
                  ? min(TERRAIN_RANGE_M, shadow_profile_cascade_end(shadow_source<false>(),
                                                                    SUN_SHADOW_CASCADE_COUNT - 1) *
                                             shadow_ray.inverse_scalar_rate)
                  : TERRAIN_RANGE_M;
    for (var z: uint = 0; z < TERRAIN_DEPTH; ++z)
    {
        let t = float(z + 1u) / float(TERRAIN_DEPTH);
        let edge = min(end, TERRAIN_RANGE_M * 0.001 * t * t);
        let start = max(previous, entry);
        if (edge > start)
        {
            let dt = (edge - start) * 0.5;
            for (var step: uint = 0; step < 2; ++step)
            {
                let a = start + float(step) * dt;
                let d = a + dt * 0.5;
                let h = height_at(radial, d);
                let inverse_radius = 1.0 / (p.planet_radius_km + h);
                let mu = (radial_dot + ray_squared * d) * inverse_radius;
                let mus = (sun_radial_dot + ray_sun_dot * d) * inverse_radius;
                let stencil = medium_profile_interval(h);
                let extinction = extinction_at_interval(stencil);
                let vis = shadow_visible_interval(shadow_source<false>(), shadow_ray, a * 1000.0,
                                                  (a + dt) * 1000.0);
                // 有限表面只需要受光积分。只有可达近场末端的射线还要给天空提供遮挡积分。
                if (vis > 0 || first_hit >= TERRAIN_RANGE_M)
                {
                    let c = scattering_source_coefficients<true>(h, stencil, phase);
                    var source : float4 = c.solar_phase_weighted *
                                          sun_transmittance(p, tau_read, linear_sampler, h, mus) *
                                          p.solar_irradiance_spectrum;
                    source += indirect(h, mu, mus, angle, c.rayleigh_scattering,
                                       c.normalization_scattering);
                    let segment = segment_transport(extinction, dt);
                    let energy = transmission * source * segment.integral;
                    lit += energy * vis;
                    blocked += energy * (1. - vis);
                    transmission *= segment.transmittance;
                }
                else
                    transmission *= exp(-dt * extinction);
                let measured : float = min((a + dt) * 1000.0, covered_metres) - a * 1000.0;
                if (measured > 0)
                    boundary_visibility = saturate(vis * dt * 1000.0 / measured);
            }
        }
        previous = edge;
        terrain_light[uint3(lane, slice, z)] = float4(terrain_rgb(lit, f.source.radiance_scale), 0);
    }
    terrain_light[uint3(lane, slice, TERRAIN_DEPTH)] =
        float4(terrain_rgb(blocked, f.source.radiance_scale), boundary_visibility);
}
