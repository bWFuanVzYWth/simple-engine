import math.constants;
// 来源：Sky Tracer 9e9430d；见 crates/atmosphere/data/README.md。
import atmosphere.parameters;
import atmosphere.sampling;
using namespace atmosphere;
struct AerialView {
    column_major var matrix : float4x4;
    var settings : float4;
};
import atmosphere_bindings;
[[vk::binding(B0, 0)]]
var p : ConstantBuffer<Params>;
[[vk::binding(B1, 0)]]
var data : StructuredBuffer<float4>;
[[vk::binding(B2, 0)]]
var tau_read : Texture2D<float4>;
[[vk::binding(B3, 0)]]
var source_read : Texture2D<float4>;
[[vk::binding(B4, 0)]]
var mean_read : Texture2D<float4>;
[[vk::binding(B5, 0)]]
var ground_read : Texture2D<float4>;
[[vk::binding(B6, 0)]]
var linear_sampler : SamplerState;
[[vk::binding(B7, 0)]]
var incoming : RWStructuredBuffer<float4>;
[vk::image_format("rgba16f")]
[[vk::binding(B8, 0)]]
var source_write : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B9, 0)]]
var mean_write : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B10, 0)]]
var ground_write : RWTexture2D<float4>;
[vk::image_format("rgba16f")]
[[vk::binding(B11, 0)]]
var tau_write : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B12, 0)]]
var output : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B13, 0)]]
var output_t : RWTexture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B14, 0)]]
var sky_write : RWTexture2D<float4>;
[[vk::binding(B15, 0)]]
var sky_read : Texture2D<float4>;
[[vk::binding(B16, 0)]]
var directions : RWStructuredBuffer<float4>;
[[vk::binding(B17, 0)]]
var moments : RWStructuredBuffer<float4>;
[[vk::binding(B18, 0)]]
var high_read : Texture2D<float4>;
[vk::image_format("rgba32f")]
[[vk::binding(B19, 0)]]
var high_write : RWTexture2D<float4>;
[[vk::binding(B20, 0)]]
var sky_mapping : StructuredBuffer<float4>;
[[vk::binding(B21, 0)]]
var aerial_view : ConstantBuffer<AerialView>;
[vk::image_format("rgba16f")]
[[vk::binding(B22, 0)]]
var aerial_light : RWTexture3D<float4>;
[vk::image_format("rgba16f")]
[[vk::binding(B23, 0)]]
var aerial_t : RWTexture3D<float4>;
func dimensions(t: Texture2D<float4>)->uint2 {
    var w : uint;
    var h : uint;
    t.GetDimensions(w, h);
    return uint2(w, h);
}
func dimensions(t: RWTexture2D<float4>)->uint2 {
    var w : uint;
    var h : uint;
    t.GetDimensions(w, h);
    return uint2(w, h);
}
func dimensions(t: RWTexture3D<float4>)->uint3 {
    var w : uint;
    var h : uint;
    var d : uint;
    t.GetDimensions(w, h, d);
    return uint3(w, h, d);
}

func phase_power(cosine: float)->float {
    return pow(max((1.0 - cosine) * 0.5, 0.0), 1.0 / 3.0);
}
func phase_coord(cosine: float, power: float)->float {
    let phase_blend = p.phase_blend_weight;
    return phase_blend * acos(cosine) * math::frac_1_pi + (1.0 - phase_blend) * power;
}
func shape(height_index: uint, sun_index: uint, uv: float2)->float4 {
    let pixel =
        float2(float(sun_index * p.phase_node_count), float(height_index * p.cone_node_count)) +
        uv * float2(float(p.phase_node_count - 1u), float(p.cone_node_count - 1u)) + 0.5;
    return source_read.SampleLevel(linear_sampler, pixel / float2(dimensions(source_read)), 0.0);
}
func log_mean(height_index: uint, sun_index: uint)->float4 {
    return mean_read.Load(int3(int2(int(sun_index), int(height_index)), 0));
}
func high_basis(radial_cosine: float, sun_radial_cosine: float, ray_sun_cosine: float)->float4 {
    let clamped_radial_cosine = clamp(radial_cosine, -1.0, 1.0);
    let tangent_radius = sqrt(max(1.0 - clamped_radial_cosine * clamped_radial_cosine, 0.0));

    let tangent_component = clamp((ray_sun_cosine - clamped_radial_cosine * sun_radial_cosine) /
                                      sqrt(max(1.0 - sun_radial_cosine * sun_radial_cosine, 1e-20)),
                                  -tangent_radius, tangent_radius);
    return float4(1.0, clamped_radial_cosine * clamped_radial_cosine,
                  tangent_component * tangent_component, clamped_radial_cosine * tangent_component);
}
func high_shape(height_index: uint, sun_index: uint, basis: float4)->float4 {
    let pixel = int2(int(sun_index * 5u), int(height_index + 1u - p.low_height_count));
    var numerator = high_read.Load(int3(pixel, 0));
    numerator = fma(high_read.Load(int3(pixel + int2(1, 0), 0)), basis.y, numerator);
    numerator = fma(high_read.Load(int3(pixel + int2(2, 0), 0)), basis.z, numerator);
    numerator = fma(high_read.Load(int3(pixel + int2(3, 0), 0)), basis.w, numerator);
    let denominator = dot(high_read.Load(int3(pixel + int2(4, 0), 0)), basis);
    return numerator / max(denominator, 1e-20);
}
struct IndirectRayGeometry {
    var ray_sun_cosine : float;
    var ray_sun_sine_squared : float;
    var phase_coordinate : float;
};
func use_high_shape(height_km: float)->bool {
    return height_km >= 35.0 && p.low_height_count < p.source_height_count;
}
func indirect(height_km: float, radial_cosine: float, sun_radial_cosine: float,
              angle: IndirectRayGeometry, rayleigh_scattering: float4,
              normalization_scattering: float4)
    ->float4 {
    var lower_height_index = 0u;
    var upper_height_index = p.source_height_count - 1u;
    while (upper_height_index - lower_height_index > 1u) {
        let middle_height_index = (lower_height_index + upper_height_index) / 2u;
        if (data[middle_height_index].x <= height_km) {
            lower_height_index = middle_height_index;
        } else {
            upper_height_index = middle_height_index;
        }
    }
    let lower_height_km = data[lower_height_index].x;
    let upper_height_km = data[upper_height_index].x;
    let height_fraction =
        clamp((height_km - lower_height_km) / (upper_height_km - lower_height_km), 0.0, 1.0);
    let elevation = asin(clamp(sun_radial_cosine, -1.0, 1.0));
    let lower_sun_coordinate =
        solar_layer(lower_height_index, elevation) * float(p.sun_state_count - 1u);
    let upper_sun_coordinate =
        solar_layer(upper_height_index, elevation) * float(p.sun_state_count - 1u);
    let lower_sun_index = min(uint(lower_sun_coordinate), p.sun_state_count - 2u);
    let upper_sun_index = min(uint(upper_sun_coordinate), p.sun_state_count - 2u);
    let lower_sun_fraction = lower_sun_coordinate - float(lower_sun_index);
    let upper_sun_fraction = upper_sun_coordinate - float(upper_sun_index);
    let brightness =
        exp(lerp(lerp(log_mean(lower_height_index, lower_sun_index),
                      log_mean(lower_height_index, lower_sun_index + 1u), lower_sun_fraction),
                 lerp(log_mean(upper_height_index, upper_sun_index),
                      log_mean(upper_height_index, upper_sun_index + 1u), upper_sun_fraction),
                 height_fraction));
    if (use_high_shape(height_km)) {
        let basis = high_basis(radial_cosine, sun_radial_cosine, angle.ray_sun_cosine);
        let source_shape = lerp(
            lerp(high_shape(lower_height_index, lower_sun_index, basis),
                 high_shape(lower_height_index, lower_sun_index + 1u, basis), lower_sun_fraction),
            lerp(high_shape(upper_height_index, upper_sun_index, basis),
                 high_shape(upper_height_index, upper_sun_index + 1u, basis), upper_sun_fraction),
            height_fraction);
        return max(source_shape * brightness * rayleigh_scattering, float4(0.0));
    }
    let cone_cosine = clamp(
        (radial_cosine - sun_radial_cosine * angle.ray_sun_cosine) /
            sqrt(max((1.0 - sun_radial_cosine * sun_radial_cosine) * angle.ray_sun_sine_squared,
                     1e-20)),
        -1.0, 1.0);
    let cone_coordinate = (1.0 - cone_cosine) * 0.5;
    let warped = cone_coordinate + p.cone_warp * cone_coordinate * (1.0 - cone_coordinate) *
                                       (2.0 * cone_coordinate - 1.0);
    let angular_coordinate = float2(angle.phase_coordinate, warped);
    let source_shape =
        lerp(lerp(shape(lower_height_index, lower_sun_index, angular_coordinate),
                  shape(lower_height_index, lower_sun_index + 1u, angular_coordinate),
                  lower_sun_fraction),
             lerp(shape(upper_height_index, upper_sun_index, angular_coordinate),
                  shape(upper_height_index, upper_sun_index + 1u, angular_coordinate),
                  upper_sun_fraction),
             height_fraction);
    return max(source_shape * brightness * normalization_scattering, float4(0.0));
}
func ground_light(sun_radial_cosine: float)->float4 {
    let sun_coordinate =
        solar_layer(0u, asin(clamp(sun_radial_cosine, -1.0, 1.0))) * float(p.sun_state_count - 1u);
    let sun_index = min(uint(sun_coordinate), p.sun_state_count - 2u);
    return lerp(ground_read.Load(int3(int2(int(sun_index), 0), 0)),
                ground_read.Load(int3(int2(int(sun_index + 1u), 0), 0)),
                sun_coordinate - float(sun_index));
}
func write_optical_depth(id: uint3)->void {
    if (id.x >= p.optical_angle_count || id.y >= p.optical_height_count) {
        return;
    }
    let height_km = data[p.optical_node_offset + id.y].x;
    let angular_coordinate = float(id.x) / float(p.optical_angle_count - 1u);
    let angular_fraction = angular_coordinate * angular_coordinate * angular_coordinate;
    let radial_cosine = horizon(p, height_km) + (1.0 - horizon(p, height_km)) * angular_fraction;
    let path_length_km = boundary(p, height_km, radial_cosine, false);
    let step_length_km = path_length_km / float(p.optical_integration_steps);
    let radial = radial_ray(p, height_km, radial_cosine);
    var optical_depth = float4(0.0);
    for (var j = 0u; j < p.optical_integration_steps; j++) {
        optical_depth += extinction_at_interval(medium_profile_interval(
                             height_at(radial, (float(j) + 0.5) * step_length_km))) *
                         step_length_km;
    }
    tau_write[int2(id.xy)] = min(optical_depth, float4(80.0));
}
[shader("compute")]
[numthreads(8, 8, 1)]
func optical_depth(uint3 id: SV_DispatchThreadID)->void {
    write_optical_depth(id);
}

[shader("compute")]
[numthreads(8, 8, 1)]
func optical_depth_rows(uint3 id: SV_DispatchThreadID)->void {
    if (id.y >= p.optical_row_count) {
        return;
    }
    write_optical_depth(uint3(id.x, id.y + p.optical_row_start, 0u));
}
[shader("compute")]
[numthreads(64, 1, 1)]
func prepare_directions(uint3 id: SV_DispatchThreadID)->void {
    if (id.x >= p.incident_direction_count || id.y >= p.batch_height_count * p.sun_state_count) {
        return;
    }
    let state = data[p.sun_state_offset + p.batch_height_start * p.sun_state_count + id.y];
    let h = state.x;
    let mu_s = state.y;
    let sun = float3(0.0, mu_s, state.z);
    let n = p.incident_direction_count / 3u;
    var v : float3 = (float3)0;
    var w : float = (float)0;
    let sample_index = select(id.x < n, id.x, n + id.x % n);
    let q = data[p.integration_angle_offset + sample_index];
    if (q.w == 0.0) {
        directions[id.y * p.incident_direction_count + id.x] = float4(0.0);
        return;
    }
    if (id.x < n) {
        let tangent = float3(0.0, sun.z, -sun.y);
        v = q.x * tangent + q.y * float3(1.0, 0.0, 0.0) + q.z * sun;
        w = q.w;
    } else {
        let hor = horizon(p, h);
        var low = -1.0;
        var high = hor;
        if (id.x >= 2u * n) {
            low = hor;
            high = 1.0;
        }
        let square = q.x * q.x;
        let u = square * q.x;
        var mu = lerp(hor, low, u);
        if (id.x >= 2u * n) {
            mu = lerp(hor, high, u);
        }
        let r = sqrt(max(1.0 - mu * mu, 0.0));
        v = float3(r * q.z, mu, r * q.y);
        w = q.w * (high - low) * 3.0 * square;
    }
    let d = 0.0004 + max(1.0 - dot(v, sun), 0.0);
    let sw = p.solar_importance_width / (p.solar_importance_width + d * d) *
             lerp(1.0, p.high_sun_weight, smoothstep(12.0, 35.0, h));
    w *= select(id.x < n, sw, 1.0 - sw);

    directions[id.y * p.incident_direction_count + id.x] = float4(v, w);
}
func trace_incident_core<let multiple_scattering : bool>(id: uint3)->void {
    if (id.x >= p.incident_direction_count || id.y >= p.batch_height_count * p.sun_state_count) {
        return;
    }
    let state_index = p.batch_height_start * p.sun_state_count + id.y;
    let state = data[p.sun_state_offset + state_index];
    let q = directions[id.y * p.incident_direction_count + id.x];
    if (q.w == 0.0) {
        incoming[id.y * p.incident_direction_count + id.x] = float4(0.0);
        return;
    }
    let sun = float3(0.0, state.y, state.z);
    let index = id.y * p.incident_direction_count + id.x;
    let result =
        integrate_core<multiple_scattering, true>(q.xyz, sun, state.x, 0.0, p.integration_steps);
    let light = result.light;
    incoming[index] = light;
}
[shader("compute")]
[numthreads(64, 1, 1)]
func trace_incident_first(uint3 id: SV_DispatchThreadID)->void {
    trace_incident_core<false>(id);
}
[shader("compute")]
[numthreads(64, 1, 1)]
func trace_incident_multiple(uint3 id: SV_DispatchThreadID)->void {
    trace_incident_core<true>(id);
}

[shader("compute")]
[numthreads(64, 1, 1)]
func incident_moments(uint3 id: SV_DispatchThreadID)->void {
    if (id.x >= p.batch_height_count * p.sun_state_count) {
        return;
    }
    let state_index = p.batch_height_start * p.sun_state_count + id.x;
    var m : float4[7] = { (float4)0, (float4)0, (float4)0, (float4)0,
                          (float4)0, (float4)0, (float4)0 };
    for (var j = 0u; j < p.incident_direction_count; j++) {
        let q = directions[id.x * p.incident_direction_count + j];
        let l = incoming[id.x * p.incident_direction_count + j] * q.w;
        let square = q.xyz * q.xyz;
        let cross = q.y * q.z;
        m[0] += l;
        m[1] += l * square.x;
        m[2] += l * square.y;
        m[3] += l * square.z;
        m[4] += l * cross;
        m[5] += q.w * float4(1.0, square);
        m[6].x += q.w * cross;
    }
    for (var k = 0u; k < 7u; k++) {
        moments[id.x * 7u + k] = m[k];
    }
    let hi = state_index / p.sun_state_count;
    let scale = max(m[0] * math::frac_1_4_pi, float4(1e-30));
    if (hi + 1u >= p.low_height_count) {
        let inverse_scale = 1.0 / scale;
        let packed : float4[5] = { (m[0] + m[1]) * inverse_scale, (m[2] - m[1]) * inverse_scale,
                                   (m[3] - m[1]) * inverse_scale, 2.0 * m[4] * inverse_scale,
                                   float4(m[5].x + m[5].y, m[5].z - m[5].y, m[5].w - m[5].y,
                                          2.0 * m[6].x) };
        for (var k = 0u; k < 5u; k++) {
            high_write[int2(int((id.x % p.sun_state_count) * 5u + k),
                            int(hi + 1u - p.low_height_count))] = packed[k];
        }
    }
    mean_write[int2(int(state_index % p.sun_state_count), int(hi))] = log(scale);
}
[shader("compute")]
[numthreads(8, 8, 1)]
func scattering_source(uint3 id: SV_DispatchThreadID)->void {
    let pixel = id.xy + uint2(0u, p.batch_height_start * p.cone_node_count);
    if (any(pixel >= dimensions(source_write)) ||
        id.y >= p.batch_height_count * p.cone_node_count) {
        return;
    }
    let hi = pixel.y / p.cone_node_count;
    let si = id.x / p.phase_node_count;
    let state_index = hi * p.sun_state_count + si;
    let local_state = (hi - p.batch_height_start) * p.sun_state_count + si;
    let state = data[p.sun_state_offset + state_index];
    let h = state.x;
    let mu_s = state.y;
    let nu = data[p.phase_node_offset + id.x % p.phase_node_count].x;
    let cone = data[p.cone_node_offset + id.y % p.cone_node_count];
    let sun = float3(0.0, mu_s, state.z);
    let tangent = float3(0.0, sun.z, -sun.y);
    let v =
        sun * nu + sqrt(max(1.0 - nu * nu, 0.0)) * (tangent * cone.x + float3(cone.y, 0.0, 0.0));
    let coeff = p.coefficient_offset + hi * 6u;
    let mi = local_state * 7u;
    let square = v * v;
    let cross = 2.0 * v.y * v.z;
    let ray = moments[mi] + moments[mi + 1u] * square.x + moments[mi + 2u] * square.y +
              moments[mi + 3u] * square.z + moments[mi + 4u] * cross;
    let weights = moments[mi + 5u];
    let normalization = weights.x + dot(weights.yzw, square) + moments[mi + 6u].x * cross;
    var source = data[coeff] * ray / max(normalization, 1e-20);

    if (h < 35.0) {
        var sums : float4[4] = { (float4)0, (float4)0, (float4)0, (float4)0 };
        var norms : float4[4] = { (float4)0, (float4)0, (float4)0, (float4)0 };

        var species_enabled : bool[4] = { (bool)0, (bool)0, (bool)0, (bool)0 };
        for (var species = 0u; species < 4u; species++) {
            species_enabled[species] = any(data[coeff + species + 1u] != float4(0.0));
        }
        for (var j = 0u; j < p.incident_direction_count; j++) {
            let q = directions[local_state * p.incident_direction_count + j];
            let l = incoming[local_state * p.incident_direction_count + j];
            let a = q.y * v.y + q.z * v.z;
            let b = q.x * v.x;

            let positive = phase_interval(clamp(a + b, -1.0, 1.0));
            let negative = phase_interval(clamp(a - b, -1.0, 1.0));
            for (var species = 0u; species < 4u; species++) {
                if (!species_enabled[species]) {
                    continue;
                }
                let weight =
                    (tabulated_phase(positive, species) + tabulated_phase(negative, species)) *
                    (0.5 * q.w);
                sums[species] += l * weight;
                norms[species] += weight;
            }
        }

        for (var species = 0u; species < 4u; species++) {
            source += data[coeff + species + 1u] * data[p.phase_mass_offset + species] *
                      sums[species] / max(norms[species], float4(1e-20));
        }
    }
    let sigma = data[coeff + 5u];
    let mean = exp(log_mean(hi, si));
    source_write[int2(pixel)] = max(source / max(mean * sigma, float4(1e-30)), float4(0.0));
}
func ground_irradiance_core<let include_incoming : bool>(id: uint3)->void {
    if (id.x >= p.sun_state_count || p.batch_height_start != 0u) {
        return;
    }
    let mu_s = data[p.sun_state_offset + id.x].y;
    var irradiance = sun_transmittance(p, tau_read, linear_sampler, 0.0, mu_s) * max(mu_s, 0.0) *
                     p.solar_irradiance_spectrum;
    if (include_incoming) {
        for (var j = 0u; j < p.incident_direction_count; j++) {
            let q = directions[id.x * p.incident_direction_count + j];
            irradiance += incoming[id.x * p.incident_direction_count + j] * (q.w * max(q.y, 0.0));
        }
    }
    ground_write[int2(int(id.x), 0)] = irradiance * (p.ground_albedo / math::pi);
}
[shader("compute")]
[numthreads(64, 1, 1)]
func ground_direct(uint3 id: SV_DispatchThreadID)->void {
    ground_irradiance_core<false>(id);
}
[shader("compute")]
[numthreads(64, 1, 1)]
func ground_multiple(uint3 id: SV_DispatchThreadID)->void {
    ground_irradiance_core<true>(id);
}

func solar_layer(layer: uint, e: float)->float {
    let offset = p.solar_node_offset + 4u * layer;
    let base = data[offset];
    let d = max(math::frac_pi_2 - e, 0.0);
    var y = base.x * e + base.y - base.z * sqrt(d / (d + 0.0872664626));
    for (var j = 1u; j <= 3u; j++) {
        let c = data[offset + j];
        let x = e - c.x;
        y += c.z * x / (abs(x) + c.y);
    }
    return clamp(y, 0.0, 1.0);
}
struct ScatteringSource {
    var solar_phase_weighted : float4;
    var rayleigh_scattering : float4;
    var normalization_scattering : float4;
};
struct MediumProfileInterval {
    var lower_offset : uint;
    var upper_offset : uint;
    var fraction : float;
};
func medium_profile_interval(height_km: float)->MediumProfileInterval {
    var lower_index = 0u;
    var upper_index = p.profile_height_count - 1u;
    while (upper_index - lower_index > 1u) {
        let middle_index = (lower_index + upper_index) / 2u;
        if (data[p.profile_offset + 7u * middle_index].x <= height_km) {
            lower_index = middle_index;
        } else {
            upper_index = middle_index;
        }
    }
    let lower_offset = p.profile_offset + lower_index * 7u;
    let upper_offset = p.profile_offset + upper_index * 7u;
    let height_fraction =
        clamp((height_km - data[lower_offset].x) / (data[upper_offset].x - data[lower_offset].x),
              0.0, 1.0);
    return { lower_offset, upper_offset, height_fraction };
}
func extinction_at_interval(stencil: MediumProfileInterval)->float4 {
    return lerp(data[stencil.lower_offset + 1u], data[stencil.upper_offset + 1u], stencil.fraction);
}
func scattering_source_coefficients<let multiple_scattering : bool>(height_km: float,
                                                                    stencil: MediumProfileInterval,
                                                                    phase: float4[5])
    ->ScatteringSource {
    var result : ScatteringSource;
    result.rayleigh_scattering =
        lerp(data[stencil.lower_offset + 2u], data[stencil.upper_offset + 2u], stencil.fraction);
    result.solar_phase_weighted = result.rayleigh_scattering * phase[0];
    result.normalization_scattering = result.rayleigh_scattering;
    let total_scattering = multiple_scattering && !use_high_shape(height_km);
    for (var j = 1u; j < 5u; j++) {
        let scattering = lerp(data[stencil.lower_offset + 2u + j],
                              data[stencil.upper_offset + 2u + j], stencil.fraction);
        result.solar_phase_weighted += scattering * phase[j];
        if (total_scattering)
            result.normalization_scattering += scattering;
    }
    return result;
}
struct PhaseInterval {
    var lower_node : uint;
    var fraction : float;
};
func phase_interval(scattering_cosine: float)->PhaseInterval {
    return phase_interval_from_power(phase_power(scattering_cosine));
}
func phase_interval_from_power(phase_power_coordinate: float)->PhaseInterval {
    let x = clamp(phase_power_coordinate * 1024.0 - 0.5, 0.0, 1023.0);
    let i = min(uint(x), 1022u);
    return PhaseInterval(i, x - float(i));
}
func tabulated_phase(stencil: PhaseInterval, species: uint)->float4 {
    let offset = p.phase_offset + species * 1024u + stencil.lower_node;
    return lerp(data[offset], data[offset + 1u], stencil.fraction);
}
struct Transport {
    var light : float4;
    var transmittance : float4;
};
// 四波长共同积分核；include_ground 仅控制末端理想行星表面的边界辐射。
// Aerial 组合显式场景表面，因此传 false；求解器与 SkyView 保留源边界条件 true。
func integrate_core<let multiple_scattering : bool, let include_ground : bool>(
    ray: float3, sun: float3, initial_height: float, segment: float, steps: uint)
    ->Transport {
    var result : Transport = (Transport)0;
    result.light = float4(0.0);
    result.transmittance = float4(1.0);
    var h = initial_height;
    var mu = ray.y;
    var mu_s = sun.y;
    let nu = dot(ray, sun);
    var entry = 0.0;
    var up = float3(0.0, 1.0, 0.0);
    if (h > p.atmosphere_height_km) {
        let radius = p.planet_radius_km + h;
        let b = radius * mu;
        let c =
            (h - p.atmosphere_height_km) * (radius + p.planet_radius_km + p.atmosphere_height_km);
        let disc = b * b - c;
        if (b >= 0.0 || disc <= 0.0) {
            return result;
        }
        entry = c / (-b + sqrt(disc));
        if (segment > 0.0 && entry >= segment) {
            return result;
        }
        up = normalize(up * radius + ray * entry);
        h = p.atmosphere_height_km;
        mu = dot(ray, up);
        mu_s = dot(sun, up);
    }
    let hit = ground(p, h, mu);
    let full_length = boundary(p, h, mu, hit);
    var length = full_length;
    if (segment > 0.0) {
        length = min(length, max(segment - entry, 0.0));
    }
    let count = steps;

    let radial = radial_ray(p, h, mu);
    let b = radial.twice_radial_dot * 0.5;
    let sun_radial_dot = radial.radius * mu_s;
    let ray_squared = dot(ray, ray);
    let middle = clamp(-b, 0.0, length);
    let hmin = height_at(radial, middle);
    let hend = height_at(radial, length);
    let scale = max(p.path_height_scale_km, 1e-10);
    let before = log(1.0 + max(h - hmin, 0.0) / scale);
    let after = log(1.0 + max(hend - hmin, 0.0) / scale);
    var split = uint(round(float(count) * before / max(before + after, 1e-20)));
    if (middle <= 0.0) {
        split = 0u;
    } else if (middle >= length) {
        split = count;
    } else {
        split = clamp(split, 1u, count - 1u);
    }
    let inverse_count = 1.0 / float(count);
    let inverse_split = 1.0 / max(float(split), 1.0);
    let inverse_tail = 1.0 / max(float(count - split), 1.0);
    var solar_phase : float4[5] = { (float4)0, (float4)0, (float4)0, (float4)0, (float4)0 };
    let cosine = clamp(nu, -1.0, 1.0);
    let power = phase_power(cosine);
    let stencil = phase_interval_from_power(power);
    let angle : IndirectRayGeometry = { nu, 1.0 - nu * nu,
                                        multiple_scattering ? phase_coord(cosine, power) : 0.0 };
    solar_phase[0] = float4((3.0 / (16.0 * math::pi)) * (1.0 + cosine * cosine));
    for (var k = 0u; k < 4u; k++) {
        solar_phase[k + 1u] = tabulated_phase(stencil, k);
    }
    var previous = 0.0;
    for (var i = 1u; i <= count; i++) {
        var edge = length * (float(i) * inverse_count);
        if (before + after >= 1e-5 && i < count) {
            if (i == split) {
                edge = middle;
            } else {
                let incoming = i < split;
                var lh = after * (float(i - split) * inverse_tail);
                if (incoming) {
                    lh = before * (1.0 - float(i) * inverse_split);
                }
                let hh = hmin + scale * (exp(lh) - 1.0);
                let c = (h - hh) * (2.0 * p.planet_radius_km + h + hh);
                let root = sqrt(max(b * b - c, 0.0));
                if (incoming) {
                    edge = c / max(-b + root, 1e-20);
                } else if (b > 0.0) {
                    edge = -c / max(root + b, 1e-20);
                } else {
                    edge = -b + root;
                }
                edge = clamp(edge, previous, length);
            }
        }
        let dt = edge - previous;
        let d = (edge + previous) * 0.5;
        previous = edge;
        let hp = height_at(radial, d);
        let inverse_radius = 1.0 / (p.planet_radius_km + hp);
        let local_mu = (b + ray_squared * d) * inverse_radius;
        let local_mu_s = (sun_radial_dot + nu * d) * inverse_radius;
        let stencil = medium_profile_interval(hp);
        let extinction = extinction_at_interval(stencil);
        let c = scattering_source_coefficients<multiple_scattering>(hp, stencil, solar_phase);
        let direct = c.solar_phase_weighted *
                     sun_transmittance(p, tau_read, linear_sampler, hp, local_mu_s) *
                     p.solar_irradiance_spectrum;
        var source = direct;

        if (multiple_scattering)
            source += indirect(hp, local_mu, local_mu_s, angle, c.rayleigh_scattering,
                               c.normalization_scattering);
        let segment = segment_transport(extinction, dt);
        result.light += result.transmittance * source * segment.integral;
        result.transmittance *= segment.transmittance;
    }
    if (include_ground && hit && length >= full_length) {
        let end_mu_s = (sun_radial_dot + nu * length) / p.planet_radius_km;
        result.light += result.transmittance * ground_light(end_mu_s);
    }
    return result;
}

func integrate_at(ray: float3, sun: float3, initial_height: float, segment: float, steps: uint)
    ->Transport {
    return integrate_core<true, true>(ray, sun, initial_height, segment, steps);
}

func integrate(ray: float3, sun: float3)->Transport {
    return integrate_at(ray, sun, p.observer_height_km, p.segment_length_km, p.integration_steps);
}
func world_sun()->float3 {
    var azimuth_sine : float;
    var azimuth_cosine : float;
    var elevation_sine : float;
    var elevation_cosine : float;
    sincos(p.sun_azimuth, azimuth_sine, azimuth_cosine);
    sincos(p.sun_elevation, elevation_sine, elevation_cosine);
    return float3(azimuth_sine * elevation_cosine, elevation_sine,
                  azimuth_cosine * elevation_cosine);
}
[shader("compute")]
[numthreads(8, 8, 1)]
func build_sky(uint3 id: SV_DispatchThreadID)->void {
    if (any(id.xy >= uint2(p.sky_width, p.sky_height))) {
        return;
    }
    let lower_rows = p.lower_sky_rows;
    var chart = 0u;
    var start = 0u;
    var rows = lower_rows;
    if (id.y >= p.sky_rows) {
        chart = 2u;
        start = p.sky_rows;
        rows = p.sky_height - p.sky_rows;
    } else if (id.y >= lower_rows) {
        chart = 1u;
        start = lower_rows;
        rows = p.sky_rows - lower_rows;
    }
    if (chart == 1u && p.observer_height_km >= p.atmosphere_height_km) {
        sky_write[int2(id.xy)] = float4(log(float3(1e-30)), 0.0);
        return;
    }
    let hit = chart == 2u;
    let u = float(id.x) / float(p.sky_width - 1u);
    let square = u * u;
    let a = square * square;
    let cp = 1.0 - 2.0 * a;

    var mu = sky_mapping[12u + id.y].x;
    let hor = horizon(p, p.observer_height_km);
    if (hit) {
        mu = min(mu, hor - 1e-7);
    } else {
        mu = max(mu, hor + 1e-7);
    }
    let r = sqrt(max(1.0 - mu * mu, 0.0));
    let ray = float3(r * 2.0 * sqrt(max(a * (1.0 - a), 0.0)), mu, r * cp);
    let sun = float3(0.0, sin(p.sun_elevation), cos(p.sun_elevation));
    let result = integrate(ray, sun);
    var rgb = float3(0.0);
    for (var k = 0u; k < 4u; k++) {
        rgb += result.light[k] * p.spectral_to_rec2020[k].xyz;
    }
    sky_write[int2(id.xy)] = float4(log(max(rgb, float3(1e-30))), 0.0);
}

[shader("compute")]
[numthreads(4, 4, 4)]
func build_aerial(uint3 id: SV_DispatchThreadID)->void {
    let dims = dimensions(aerial_light);
    if (any(id >= dims))
        return;
    let uv : float2 = (float2(id.xy) + .5) / float2(dims.xy);
    let ray
        : float3 =
              normalize(mul(aerial_view.matrix, float4(uv.x * 2. - 1., 1. - uv.y * 2., 0, 1)).xyz);
    let z : float = (float(id.z) + .5) / float(dims.z);
    let result = integrate_core<true, false>(ray, world_sun(), p.observer_height_km,
                                             aerial_view.settings.x * z * z, p.integration_steps);
    var light : float3 = 0;
    var clear : float3 = 0;
    var filtered : float3 = 0;
    for (var k: uint = 0; k < 4; k++) {
        let solar_rgb = p.solar_irradiance_spectrum[k] * p.spectral_to_rec2020[k].xyz;
        light += result.light[k] * p.spectral_to_rec2020[k].xyz;
        clear += solar_rgb;
        filtered += result.transmittance[k] * solar_rgb;
    }
    aerial_light[id] = float4(max(light * aerial_view.settings.yzw, 0), 1);
    aerial_t[id] = float4(clamp(filtered / max(clear, 1e-20), 0, 1), 1);
}
