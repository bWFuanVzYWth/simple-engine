import atmosphere.terrain_frame;
#include "shadow_profile.inc.slang"
[[vk::binding(8, 0)]]
StructuredBuffer<TerrainFrame> frames;
[[vk::binding(9, 0)]]
RWStructuredBuffer<uint4> shadowCells;
[[vk::binding(10, 0)]]
RWStructuredBuffer<uint> previousProfiles;
[[vk::binding(11, 0)]]
[vk::image_format("rgba16f")]
RWTexture3D<float4> terrain_light;
[[vk::binding(12, 0)]]
StructuredBuffer<uint> first_hits;
#include "shadow_source.inc.slang"
groupshared uint profileChanged;
groupshared uint sameView;
bool updateShadowProfile(uint slice, uint lane)
{
    uint start = slice * (SUN_SHADOW_PROFILE_TOTAL_CAPACITY + SUN_SHADOW_CASCADE_COUNT + 1u);
    if (lane == 0)
    {
        sameView = !terrain_full(frames[0]) && previousProfiles[start] == frames[0].source.state.z;
        previousProfiles[start] = frames[0].source.state.z;
        profileChanged = sameView == 0 ? 1u : 0u;
    }
    GroupMemoryBarrierWithGroupSync();
    bool reuse = sameView != 0;
    start += 1;
    bool changed = (first_hits[slice * TERRAIN_RADIAL + lane] >> 31u) != 0;
    if (lane < SUN_SHADOW_CASCADE_COUNT)
    {
        uint count = asuint(sunShadowProfileCounts[lane]);
        if (reuse)
            changed = changed || previousProfiles[start + lane] != count;
        previousProfiles[start + lane] = count;
    }
    for (int cascade = 0; cascade < SUN_SHADOW_CASCADE_COUNT; cascade++)
    {
        int base = sunShadowProfileBase(cascade);
        for (int i = int(lane); i < sunShadowProfileCounts[cascade]; i += 128)
        {
            uint index = start + SUN_SHADOW_CASCADE_COUNT + base + i;
            uint depth = asuint(sunShadowProfileEntries[base + i].y);
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

float3 terrain_rgb(float4 spectrum, float3 scale)
{
    float3 rgb = 0;
    for (uint k = 0; k < 4; ++k)
        rgb += spectrum[k] * p.rgb[k].xyz;
    return max(rgb * scale, 0);
}

// 各深度段沿相机可见的空气积分受光源项。末层单独保存天空需扣除的遮挡量及远场边界可见度。
[shader("compute")]
[numthreads(128, 1, 1)]
void terrain_integrate(uint3 group: SV_GroupID, uint lane: SV_GroupIndex)
{
    uint slice = terrain_slice_index(frames[0], group.y);
    terrain_slice(lane, slice);
    if (!updateShadowProfile(slice, lane))
        return;
    TerrainFrame f = frames[0];
    float3 ray = terrain_ray(f, lane, slice), sun = f.source.sun.xyz;
    float height = f.source.camera.w;
    float radius = p.sun.w + height;
    float first_hit = asfloat(first_hits[slice * TERRAIN_RADIAL + lane] & 0x7fffffffu);
    float entry = 0, end = first_hit * 0.001;
    float entry_height = height, entry_mu = ray.y;
    if (height > p.medium.x)
    {
        float b = radius * ray.y, c = (height - p.medium.x) * (radius + p.sun.w + p.medium.x);
        float disc = b * b - c;
        if (b < 0 && disc > 0)
        {
            float root = sqrt(disc);
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
    float4 phase[5];
    phase[0] = float4(3.0 * (1.0 + nu * nu) / (16.0 * atmosphere::PI));
    for (uint k = 0; k < 4; ++k)
        phase[k + 1] = tabulated_phase(stencil, k);
    float4 lit = 0, blocked = 0, transmission = 1;
    float previous = 0, boundary_visibility = 1;
    float2 projected = float2(dot(ray, sunShadowProfileBasisU), dot(ray, sunShadowProfileBasisV));
    float rate = dot(projected, sunShadowProfileDirection);
    float covered_metres =
        rate > SUN_SHADOW_PROFILE_EPSILON
            ? min(TERRAIN_RANGE_M,
                  sunShadowProfileCascadeEnd(shadowSource(), SUN_SHADOW_CASCADE_COUNT - 1) / rate)
            : TERRAIN_RANGE_M;
    for (uint z = 0; z < TERRAIN_DEPTH; ++z)
    {
        float t = float(z + 1u) / float(TERRAIN_DEPTH);
        float edge = min(end, TERRAIN_RANGE_M * 0.001 * t * t);
        float start = max(previous, entry);
        if (edge > start)
        {
            float dt = (edge - start) * 0.5;
            for (uint step = 0; step < 2; ++step)
            {
                float a = start + float(step) * dt, d = a + dt * 0.5;
                float h = height_at(p, height, ray.y, d);
                float3 up = (float3(0, radius, 0) + ray * d) / (p.sun.w + h);
                Medium c = medium(h);
                float vis =
                    localSunVisibilityInterval(shadowSource(), ray, a * 1000.0, (a + dt) * 1000.0);
                // 有限表面只需要受光积分。只有可达近场末端的射线还要给天空提供遮挡积分。
                if (vis > 0 || first_hit >= TERRAIN_RANGE_M)
                {
                    float4 direct = 0;
                    for (uint k = 0; k < 5; ++k)
                        direct += c.scattering[k] * phase[k];
                    float mus = dot(up, sun);
                    float4 source =
                        direct * sun_transmittance(p, tau_read, linear_sampler, h, mus) * p.solar;
                    if (p.solve.w != 1u)
                        source += indirect(h, dot(up, ray), mus, nu, phase_coord(nu), c);
                    float4 energy = transmission * source * segment_integral(c.extinction, dt);
                    lit += energy * vis;
                    blocked += energy * (1. - vis);
                }
                transmission *= exp(-dt * c.extinction);
                float measured = min((a + dt) * 1000.0, covered_metres) - a * 1000.0;
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
