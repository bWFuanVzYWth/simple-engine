// 来源：Sky Tracer 9e9430d；见 crates/atmosphere/data/README.md。
module sampling;
import atmosphere.parameters;
namespace atmosphere
{
/// 圆周率，与源 f32 常量一致。
public static const float PI = 3.141592653589793;
/// 原坐标拟合的六个开关位；资源所有者保持与物理节点一致。
public uint mapping_flags(Params p)
{
    return p.mapping.x;
}
/// 海拔 h 的球形地平线方向余弦；h 非负，p.sun.w 是底部半径。
public float horizon(Params p, float h)
{
    return -sqrt(max(h * (2.0 * p.sun.w + h), 0.0)) / (p.sun.w + h);
}
/// 判断径向方向余弦 mu 是否先命中地表，包含切向地面边界。
public bool ground(Params p, float h, float mu)
{
    return mu < 0.0 && mu <= horizon(p, h);
}
/// 从海拔 h 沿单位方向余弦 mu 前进 d km 后的海拔，采用避免大半径相消的形式。
public float height_at(Params p, float h, float mu, float d)
{
    let dh = d * (d + 2.0 * (p.sun.w + h) * mu);
    return max(0.0, h + dh / (sqrt(max((p.sun.w + h) * (p.sun.w + h) + dh, 0.0)) + p.sun.w + h));
}
/// 从大气内出发至地面（hit）或顶层球面的非负距离 km；hit 须等于 ground。
public float boundary(Params p, float h, float mu, bool hit)
{
    let b = (p.sun.w + h) * mu;
    if (hit)
    {
        let c = h * (2.0 * p.sun.w + h);
        return max(0.0, c / max(-b + sqrt(max(b * b - c, 0.0)), 1e-20));
    }
    let c = (p.medium.x - h) * (2.0 * p.sun.w + p.medium.x + h);
    let root = sqrt(max(b * b + c, 0.0));
    if (b > 0.0)
    {
        return max(0.0, c / max(root + b, 1e-20));
    }
    return max(0.0, -b + root);
}
/// optical 高度插值坐标；节点由同一 mapping 标定构造。
public float height_coord(Params p, float h)
{
    if ((mapping_flags(p) & 16u) != 0u)
    {
        let x = sqrt(clamp(h, 0.0, p.medium.x));
        var j = 0u;
        for (var i = 0u; i < 5u; i++)
        {
            if (x > p.optical_segments[i].x)
            {
                j = i + 1u;
            }
        }
        let c = p.optical_segments[j];
        return clamp(x * c.y + c.z, 0.0, 1.0);
    }
    let scale = sqrt(0.01 / p.medium.x);
    return 0.35 * log(1.0 + sqrt(clamp(h / p.medium.x, 0.0, 1.0)) / scale) /
               log(1.0 + 1.0 / scale) +
           0.65 * clamp(h / p.medium.x, 0.0, 1.0);
}
/// 全仰角区间的归一 atan 分布；w 为正弧度宽度。
public float atan_cdf(Params p, float e, float c, float w)
{
    let lo = atan((-PI * 0.5 - c) / w);
    return (atan((e - c) / w) - lo) / (atan((PI * 0.5 - c) / w) - lo);
}
/// 非拟合太阳坐标，与源 legacy mapping 分支一致。
public float solar_coord(Params p, float h, float mu)
{
    let e = asin(clamp(mu, -1.0, 1.0));
    let hor = asin(horizon(p, h));

    let cap_scale = 5.0 * PI / 180.0;
    let d = PI * 0.5 - e;
    let cap = 1.0 - sqrt(d / (d + cap_scale)) / sqrt(PI / (PI + cap_scale));
    return clamp(0.25 * (e / PI + 0.5) + 0.20 * atan_cdf(p, e, hor, 2.0 * PI / 180.0) +
                     0.20 * atan_cdf(p, e, hor - 6.0 * PI / 180.0, 6.0 * PI / 180.0) +
                     0.15 * atan_cdf(p, e, -hor, 0.5 * PI / 180.0) + 0.20 * cap,
                 0.0, 1.0);
}
/// 从大气内朝边界太阳的四波长 T；地表遮挡返回零，线性 clamp sampler 对应 optical LUT。
public float4 sun_transmittance(Params p, Texture2D<float4> tau_read, SamplerState linear_sampler,
                                float h, float mu)
{
    if (ground(p, h, mu))
    {
        return float4(0.0);
    }
    let x = clamp((mu - horizon(p, h)) / (1.0 - horizon(p, h)), 0.0, 1.0);
    let a = pow(x, 1.0 / 3.0);
    var u = a;
    if ((mapping_flags(p) & 16u) == 0u)
    {
        let b = pow(1.0 - x, 1.0 / 3.0);
        u = a / max(a + b, 1e-20);
    }
    let pixel = float2(u * float(p.dims.w - 1u), height_coord(p, h) * float(p.dims.z - 1u)) + 0.5;
    return exp(-tau_read.SampleLevel(linear_sampler,
                                     pixel / float2(float(p.dims.w), float(p.dims.z)), 0.0));
}
/// SkyView 三 chart 的仰角范围：低天空、上天空、地面；观察者可位于太空。
public float2 elevation_bounds(Params p, uint chart)
{
    let hor = asin(horizon(p, p.view.w));
    if (chart == 2u)
    {
        return float2(-PI * 0.5, hor);
    }
    if (chart == 1u)
    {
        return float2(0.0, PI * 0.5);
    }

    if (p.view.w >= p.medium.x)
    {
        return float2(hor, -acos(clamp((p.sun.w + p.medium.x) / (p.sun.w + p.view.w), 0.0, 1.0)));
    }
    return float2(hor, 0.0);
}
float interval_cdf(float e, float c, float w, float2 bounds)
{
    let low = atan((bounds.x - c) / w);
    return (atan((e - c) / w) - low) / max(atan((bounds.y - c) / w) - low, 1e-10);
}
/// 使用当前观察者 12 个 chart 系数求仰角坐标；mapping 后接每行方向余弦。
public float elevation_coord(Params p, StructuredBuffer<float4> sky_mapping, float e, uint chart)
{
    if ((mapping_flags(p) & 32u) != 0u)
    {
        let base = sky_mapping[4u * chart];
        let x = clamp(e, base.z, base.w);
        var y = base.x * (x - base.z);
        for (var j = 1u; j <= 3u; j++)
        {
            let c = sky_mapping[4u * chart + j];
            let d = x - c.x;
            if (c.x < base.z || c.x > base.w)
            {
                y += (x - base.z) * c.z / (abs(d) + c.y);
            }
            else
            {
                y += c.z * (d / (abs(d) + c.y) - c.w);
            }
        }
        return clamp(y, 0.0, 1.0);
    }
    let bounds = elevation_bounds(p, chart);
    let hor = asin(horizon(p, p.view.w));
    var outer = hor;
    if (p.view.w > p.medium.x)
    {
        let r = p.sun.w + p.view.w;
        let top = p.sun.w + p.medium.x;
        outer = -acos(clamp(top / r, 0.0, 1.0));
    }
    let x = clamp(e, bounds.x, bounds.y);
    return clamp(0.15 * (x - bounds.x) / max(bounds.y - bounds.x, 1e-10) +
                     0.40 * interval_cdf(x, hor, 1.0 * PI / 180.0, bounds) +
                     0.35 * interval_cdf(x, p.sun.y, 0.7 * PI / 180.0, bounds) +
                     0.10 * interval_cdf(x, outer, 0.5 * PI / 180.0, bounds),
                 0.0, 1.0);
}
/// 单位局部方向 ray 的边界辐射亮度，线性 Rec.2020，未含太阳圆盘或应用强度。
/// sky_read 为对应观察者的 log RGB SkyView；太空薄边缘采用源 chord 极限而非 log 混到真空。
public float3 sky_radiance(Params p, StructuredBuffer<float4> sky_mapping,
                           Texture2D<float4> sky_read, float3 ray)
{
    if (p.view.w >= p.medium.x)
    {
        let radius = p.sun.w + p.view.w;
        let b = radius * ray.y;
        let c = (p.view.w - p.medium.x) * (radius + p.sun.w + p.medium.x);
        if (b >= 0.0 || b * b <= c)
        {
            return float3(0.0);
        }
    }
    let cp = clamp((sin(p.sun.x) * ray.x + cos(p.sun.x) * ray.z) / max(length(ray.xz), 1e-10), -1.0,
                   1.0);
    var u = sqrt(max((1.0 - cp) * 0.5, 0.0));
    if ((mapping_flags(p) & 32u) != 0u)
    {
        u = sqrt(u);
    }
    let hit = ground(p, p.view.w, ray.y);
    let lower_rows = p.sky.z;
    var chart = 0u;
    var start = 0u;
    var rows = lower_rows;
    if (hit)
    {
        chart = 2u;
        start = p.sky.w;
        rows = p.sky.y - p.sky.w;
    }
    else if (ray.y >= 0.0)
    {
        chart = 1u;
        start = lower_rows;
        rows = p.sky.w - lower_rows;
    }
    let y = elevation_coord(p, sky_mapping, asin(clamp(ray.y, -1.0, 1.0)), chart);
    let row = float(start) + y * float(rows - 1u);
    let end = start + rows - 1u;
    let x = u * float(p.sky.x - 1u);
    let ix = min(uint(x), p.sky.x - 2u);
    let iy = clamp(uint(row), start, end - 1u);
    let a = lerp(sky_read.Load(int3(int2(int(ix), int(iy)), 0)).xyz,
                 sky_read.Load(int3(int2(int(ix + 1u), int(iy)), 0)).xyz, x - float(ix));
    let b = lerp(sky_read.Load(int3(int2(int(ix), int(iy + 1u)), 0)).xyz,
                 sky_read.Load(int3(int2(int(ix + 1u), int(iy + 1u)), 0)).xyz, x - float(ix));
    if (p.view.w >= p.medium.x && !hit && iy == lower_rows - 2u)
    {

        let previous = sky_mapping[12u + lower_rows - 2u].x;
        let previous2 = sky_mapping[12u + lower_rows - 3u].x;
        let radius = p.sun.w + p.view.w;
        let c = (p.view.w - p.medium.x) * (radius + p.sun.w + p.medium.x);
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
public float4 view_transmittance(Params p, Texture2D<float4> tau_read, SamplerState linear_sampler,
                                 float3 ray)
{
    var h = p.view.w;
    var mu = ray.y;
    if (ground(p, h, mu))
    {
        return float4(0.0);
    }
    if (h > p.medium.x)
    {
        let r = p.sun.w + h;
        let b = r * mu;
        let c = (h - p.medium.x) * (r + p.sun.w + p.medium.x);
        let d = b * b - c;
        if (b >= 0.0 || d <= 0.0)
        {
            return float4(1.0);
        }
        mu = -sqrt(d) / (p.sun.w + p.medium.x);
        h = p.medium.x;
    }
    return sun_transmittance(p, tau_read, linear_sampler, h, mu);
}

/// 常系数线段的源积分权重 (1-exp(-sigma*dt))/sigma；sigma>=0，dt>=0，单位 km。
/// 低光学厚度采用源三项展开，sigma=0 的极限为 dt；不执行完整介质积分或采样策略。
public float4 segment_integral(float4 sigma, float dt)
{
    float4 tau = sigma * dt;
    float4 transmittance = exp(-tau);
    return select(tau < float4(0.001), dt * (1.0 - tau * 0.5 + tau * tau / 6.0),
                  (1.0 - transmittance) / max(sigma, float4(1e-30)));
}

/// 线性 Rec.2020 的有限距离 L/T；T 是太阳谱加权的 RGB 近似，不能代表任意背景光谱。
public struct AerialSample
{
    /// 线段介质散射辐射亮度；已含应用强度，不含理想行星表面的末端边界辐射。
    public float3 radiance;
    /// 太阳谱投影、限于 [0,1] 的 RGB T。
    public float3 transmittance;
};

/// 采样 z² 距离体积；uv 为屏幕顶部 y=0，distance/max_distance 单位 km，最大距离为正。
/// 超出体积距离沿用末层；首层之前线性接到 d=0 的 L=0/T=1，不把相机近处误当整段雾。
public AerialSample aerial_sample(Texture3D<float4> light, Texture3D<float4> transmittance,
                                  SamplerState sampler, float2 uv, float distance,
                                  float max_distance)
{
    uint width, height, depth;
    light.GetDimensions(width, height, depth);
    float first_z = 0.5 / float(depth);
    float weight = saturate(distance / (max_distance * first_z * first_z));
    float z = max(sqrt(saturate(distance / max_distance)), first_z);
    float3 coordinate = float3(saturate(uv), z);
    AerialSample result;
    result.radiance = max(light.SampleLevel(sampler, coordinate, 0).xyz, 0) * weight;
    result.transmittance =
        lerp(float3(1), saturate(transmittance.SampleLevel(sampler, coordinate, 0).xyz), weight);
    return result;
}
}
