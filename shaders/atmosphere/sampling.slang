// 来源：Sky Tracer 9e9430d；见 crates/atmosphere/data/README.md。
/// 仅消费 Earth/balanced 的63号标定；参数与查表资源必须由同批 Model 建立。
module sampling;
import math.constants;
import atmosphere.parameters;
namespace atmosphere {
/// 海拔 h 的球形地平线方向余弦；h 非负，p.planet_radius_km 是底部半径。
public func horizon(p: Params, height_km: float)->float {
    return -sqrt(max(height_km * (2.0 * p.planet_radius_km + height_km), 0.0)) /
           (p.planet_radius_km + height_km);
}
/// 判断径向方向余弦 mu 是否先命中地表，包含切向地面边界。
public func ground(p: Params, height_km: float, radial_cosine: float)->bool {
    return radial_cosine < 0.0 && radial_cosine <= horizon(p, height_km);
}
/// 同一径向射线的固定几何量；距离使用km，twice_radial_dot为两倍半径乘方向余弦。
public struct RadialRay {
    public var height : float;
    public var radius : float;
    public var radius_squared : float;
    public var twice_radial_dot : float;
};
/// 为固定海拔h和单位方向余弦mu准备几何量；p来自同批Earth模型。
public func radial_ray(p: Params, height_km: float, radial_cosine: float)->RadialRay {
    let radius = p.planet_radius_km + height_km;
    return { height_km, radius, radius * radius, 2.0 * radius * radial_cosine };
}
/// 沿已准备的射线前进d km后的海拔；保留避免大半径相消的有理化形式。
public func height_at(ray: RadialRay, distance_km: float)->float {
    let squared_radius_change = distance_km * (distance_km + ray.twice_radial_dot);
    return max(0.0, ray.height + squared_radius_change /
                                     (sqrt(max(ray.radius_squared + squared_radius_change, 0.0)) +
                                      ray.radius));
}
/// 从海拔h沿单位方向余弦mu前进d km后的海拔；单点调用与批量射线共享数值实现。
public func height_at(p: Params, height_km: float, radial_cosine: float, distance_km: float)
    ->float {
    return height_at(radial_ray(p, height_km, radial_cosine), distance_km);
}
/// 从大气内出发至地面（hit）或顶层球面的非负距离 km；hit 须等于 ground。
public func boundary(p: Params, height_km: float, radial_cosine: float, hits_ground: bool)->float {
    let b = (p.planet_radius_km + height_km) * radial_cosine;
    if (hits_ground) {
        let c = height_km * (2.0 * p.planet_radius_km + height_km);
        return max(0.0, c / max(-b + sqrt(max(b * b - c, 0.0)), 1e-20));
    }
    let c = (p.atmosphere_height_km - height_km) *
            (2.0 * p.planet_radius_km + p.atmosphere_height_km + height_km);
    let root = sqrt(max(b * b + c, 0.0));
    if (b > 0.0) {
        return max(0.0, c / max(root + b, 1e-20));
    }
    return max(0.0, -b + root);
}
/// optical 高度插值坐标；节点由同一 mapping 标定构造。
public func height_coord(p: Params, height_km: float)->float {
    let sqrt_height = sqrt(clamp(height_km, 0.0, p.atmosphere_height_km));
    var segment_index = 0u;
    for (var i = 0u; i < 5u; i++) {
        if (sqrt_height > p.optical_height_segments[i].x) {
            segment_index = i + 1u;
        }
    }
    let segment = p.optical_height_segments[segment_index];
    return clamp(sqrt_height * segment.y + segment.z, 0.0, 1.0);
}
/// 从大气内朝边界太阳的四波长 T；地表遮挡返回零，线性 clamp sampler 对应 optical LUT。
public func sun_transmittance(p: Params, tau_read: Texture2D<float4>, linear_sampler: SamplerState,
                              height_km: float, radial_cosine: float)
    ->float4 {
    let horizon_cosine = horizon(p, height_km);
    if (radial_cosine < 0.0 && radial_cosine <= horizon_cosine) {
        return float4(0.0);
    }
    let angular_fraction =
        clamp((radial_cosine - horizon_cosine) / (1.0 - horizon_cosine), 0.0, 1.0);
    let angular_coordinate = pow(angular_fraction, 1.0 / 3.0);
    let pixel = float2(angular_coordinate * float(p.optical_angle_count - 1u),
                       height_coord(p, height_km) * float(p.optical_height_count - 1u)) +
                0.5;
    return exp(-tau_read.SampleLevel(
        linear_sampler, pixel / float2(float(p.optical_angle_count), float(p.optical_height_count)),
        0.0));
}
/// 使用当前观察者 12 个 chart 系数求仰角坐标；mapping 后接每行方向余弦。
public func elevation_coord(sky_mapping: StructuredBuffer<float4>, elevation: float, chart: uint)
    ->float {
    let base = sky_mapping[4u * chart];
    let x = clamp(elevation, base.z, base.w);
    var y = base.x * (x - base.z);
    for (var term_index = 1u; term_index <= 3u; term_index++) {
        let c = sky_mapping[4u * chart + term_index];
        let d = x - c.x;
        if (c.x < base.z || c.x > base.w) {
            y += (x - base.z) * c.z / (abs(d) + c.y);
        } else {
            y += c.z * (d / (abs(d) + c.y) - c.w);
        }
    }
    return clamp(y, 0.0, 1.0);
}
/// 单位局部方向 ray 的边界辐射亮度，线性 Rec.2020，未含太阳圆盘或应用强度。
/// sky_read 为对应观察者的 log RGB SkyView；太空薄边缘采用源 chord 极限而非 log 混到真空。
public func sky_radiance(p: Params, sky_mapping: StructuredBuffer<float4>,
                         sky_read: Texture2D<float4>, ray: float3)
    ->float3 {
    if (p.observer_height_km >= p.atmosphere_height_km) {
        let radius = p.planet_radius_km + p.observer_height_km;
        let b = radius * ray.y;
        let c = (p.observer_height_km - p.atmosphere_height_km) *
                (radius + p.planet_radius_km + p.atmosphere_height_km);
        if (b >= 0.0 || b * b <= c) {
            return float3(0.0);
        }
    }
    let cp = clamp((sin(p.sun_azimuth) * ray.x + cos(p.sun_azimuth) * ray.z) /
                       max(length(ray.xz), 1e-10),
                   -1.0, 1.0);
    let u = sqrt(sqrt(max((1.0 - cp) * 0.5, 0.0)));
    let hit = ground(p, p.observer_height_km, ray.y);
    let lower_rows = p.lower_sky_rows;
    var chart = 0u;
    var start = 0u;
    var rows = lower_rows;
    if (hit) {
        chart = 2u;
        start = p.sky_rows;
        rows = p.sky_height - p.sky_rows;
    } else if (ray.y >= 0.0) {
        chart = 1u;
        start = lower_rows;
        rows = p.sky_rows - lower_rows;
    }
    let y = elevation_coord(sky_mapping, asin(clamp(ray.y, -1.0, 1.0)), chart);
    let row = float(start) + y * float(rows - 1u);
    let end = start + rows - 1u;
    let x = u * float(p.sky_width - 1u);
    let ix = min(uint(x), p.sky_width - 2u);
    let iy = clamp(uint(row), start, end - 1u);
    let a = lerp(sky_read.Load(int3(int2(int(ix), int(iy)), 0)).xyz,
                 sky_read.Load(int3(int2(int(ix + 1u), int(iy)), 0)).xyz, x - float(ix));
    let b = lerp(sky_read.Load(int3(int2(int(ix), int(iy + 1u)), 0)).xyz,
                 sky_read.Load(int3(int2(int(ix + 1u), int(iy + 1u)), 0)).xyz, x - float(ix));
    if (p.observer_height_km >= p.atmosphere_height_km && !hit && iy == lower_rows - 2u) {
        let previous = sky_mapping[12u + lower_rows - 2u].x;
        let previous2 = sky_mapping[12u + lower_rows - 3u].x;
        let radius = p.planet_radius_km + p.observer_height_km;
        let c = (p.observer_height_km - p.atmosphere_height_km) *
                (radius + p.planet_radius_km + p.atmosphere_height_km);
        let d = max((radius * ray.y) * (radius * ray.y) - c, 0.0);
        let d0 = max((radius * previous) * (radius * previous) - c, 1e-20);
        let d1 = max((radius * previous2) * (radius * previous2) - c, d0 + 1e-10);
        let earlier =
            lerp(sky_read.Load(int3(int2(int(ix), int(iy - 1u)), 0)).xyz,
                 sky_read.Load(int3(int2(int(ix + 1u), int(iy - 1u)), 0)).xyz, x - float(ix));
        let normalized_change = exp(earlier - a) * sqrt(d0 / d1) - float3(1.0);
        return max(exp(a) * sqrt(clamp(d / d0, 0.0, 1.0)) *
                       (float3(1.0) + normalized_change * ((d - d0) / (d1 - d0))),
                   float3(0.0));
    }
    return max(exp(lerp(a, b, row - float(iy))) - float3(1e-30), float3(0.0));
}
/// 当前观察者朝单位方向的四波长 T；支持大气外入射，真空未入射为 1、地表遮挡为 0。
public func view_transmittance(p: Params, tau_read: Texture2D<float4>, linear_sampler: SamplerState,
                               ray: float3)
    ->float4 {
    var h = p.observer_height_km;
    var mu = ray.y;
    if (ground(p, h, mu)) {
        return float4(0.0);
    }
    if (h > p.atmosphere_height_km) {
        let r = p.planet_radius_km + h;
        let b = r * mu;
        let c = (h - p.atmosphere_height_km) * (r + p.planet_radius_km + p.atmosphere_height_km);
        let d = b * b - c;
        if (b >= 0.0 || d <= 0.0) {
            return float4(1.0);
        }
        mu = -sqrt(d) / (p.planet_radius_km + p.atmosphere_height_km);
        h = p.atmosphere_height_km;
    }
    return sun_transmittance(p, tau_read, linear_sampler, h, mu);
}

/// 常系数线段的四波长透射与源积分权重，均由同一次光学厚度和指数求得。
public struct SegmentTransport {
    public var transmittance : float4;
    public var integral : float4;
};
/// sigma>=0、dt>=0，单位km；低光学厚度使用同一三项展开，真空积分极限为dt。
public func segment_transport(sigma: float4, dt: float)->SegmentTransport {
    let tau = sigma * dt;
    let transmittance = exp(-tau);
    let series = dt * fma(tau, fma(tau, float4(1.0 / 6.0), float4(-0.5)), float4(1.0));
    let integral = (1.0 - transmittance) / max(sigma, float4(1e-30));
    return { transmittance, select(tau < 0.001, series, integral) };
}
/// 线性 Rec.2020 的有限距离 L/T；T 是太阳谱加权的 RGB 近似，不能代表任意背景光谱。
public struct AerialSample {
    /// 线段介质散射辐射亮度；已含应用强度，不含理想行星表面的末端边界辐射。
    public var radiance : float3;
    /// 太阳谱投影、限于 [0,1] 的 RGB T。
    public var transmittance : float3;
};

/// 采样 z² 距离体积；uv 为屏幕顶部 y=0，distance/max_distance 单位 km，最大距离为正。
/// 超出体积距离沿用末层；首层之前线性接到 d=0 的 L=0/T=1，不把相机近处误当整段雾。
public func aerial_sample(light: Texture3D<float4>, transmittance: Texture3D<float4>,
                          sampler: SamplerState, uv: float2, distance: float, max_distance: float)
    ->AerialSample {
    var width : uint;
    var height : uint;
    var depth : uint;
    light.GetDimensions(width, height, depth);
    let first_z = 0.5 / float(depth);
    let weight : float = saturate(distance / (max_distance * first_z * first_z));
    let z = max(sqrt(saturate(distance / max_distance)), first_z);
    let coordinate = float3(saturate(uv), z);
    var result : AerialSample;
    result.radiance = max(light.SampleLevel(sampler, coordinate, 0).xyz, 0) * weight;
    result.transmittance =
        lerp(float3(1), saturate(transmittance.SampleLevel(sampler, coordinate, 0).xyz), weight);
    return result;
}
/// 均匀太阳盘的法向辐照度除以投影立体角；半角在 (0, pi/2)。
/// 选择太阳方向的 PDF 仍使用未投影立体角，不能复用本归一化。
public func solar_radiance(p: Params, optical_depth: Texture2D<float4>, sampler: SamplerState,
                           direction: float3, scale: float3)
    ->float3 {
    let spectrum =
        p.solar_irradiance_spectrum * view_transmittance(p, optical_depth, sampler, direction);
    let rgb : float3 = spectrum.x * p.spectral_to_rec2020[0].xyz +
                       spectrum.y * p.spectral_to_rec2020[1].xyz +
                       spectrum.z * p.spectral_to_rec2020[2].xyz +
                       spectrum.w * p.spectral_to_rec2020[3].xyz;
    let sine = sin(p.sun_angular_radius);
    return rgb * scale / (math::pi * sine * sine);
}

/// 在同一个线性 Rec.2020 空间合成 L + T * 背景；应用决定介质准入与非负裁剪。
public func composite_aerial(sample_value: AerialSample, background: float3)->float3 {
    return sample_value.radiance + sample_value.transmittance * background;
}

}
