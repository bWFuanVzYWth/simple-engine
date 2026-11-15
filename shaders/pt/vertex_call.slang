/// 新命中材质阶段的116B逻辑字段；实际栈布局由驱动决定。
/// 几何及观察方向由实际HitObject在closest-hit中恢复。
/// 输入/输出复用同一存储，不含材质身份、纹素、法线、射线查询状态或光源采样器。
module vertex_call;
import pt.scene;
import pt.integrator;

namespace pt
{
/// 一族已经采样并查询可见性的灯；pdf=0表示不参与材质求值。
public struct DirectSample
{
    public var direction : float3;
    public var pdf : float;
    public var distance : float;
    public static func from_light(light: LightSample)->DirectSample
    {
        return { light.direction, light.pdf, light.distance };
    }
    public func light()->LightSample
    {
        var result : LightSample = {};
        result.direction = direction;
        result.pdf = pdf;
        result.distance = distance;
        return result;
    }
};

/// 输入origin_pixel保存pixel.xy/sample的位模式；输出改为新origin。
/// sun/lamp输入为DirectSample，输出由store复用为方向和两族权重，仅经result/weights读取。
/// 调用前须消费入射Beer，调用后由raygen做RR及finish；depth/events各占7bit。
public struct VertexCall
{
    public var origin_pixel : float3;
    public var throughput : float3;
    public var radiance : float3;
    public var medium : float4;
    public var previous : float3;
    public var previous_pdf : float;
    public var eta_scale : float;
    public var state : uint;
    public var sun : DirectSample;
    public var lamp : DirectSample;

    public static func input(data: TransportCall, sun: LightSample, lamp: LightSample)->VertexCall
    {
        return { asfloat(uint3(data.pixel, data.sample)),
                 data.path.throughput,
                 data.path.radiance,
                 data.path.medium,
                 data.path.previous,
                 data.path.previous_pdf,
                 data.path.eta_scale,
                 data.path.depth | (data.path.dielectric_events << 7),
                 DirectSample::from_light(sun),
                 DirectSample::from_light(lamp) };
    }
    public func unpack(hit: Hit, direction: float3)->TransportCall
    {
        var data : TransportCall = {};
        data.hit = hit;
        data.pixel = asuint(origin_pixel.xy);
        data.sample = asuint(origin_pixel.z);
        data.path.direction = direction;
        data.path.throughput = throughput;
        data.path.radiance = radiance;
        data.path.medium = medium;
        data.path.previous = previous;
        data.path.previous_pdf = previous_pdf;
        data.path.eta_scale = eta_scale;
        data.path.depth = state & 127u;
        data.path.dielectric_events = (state >> 7) & 127u;
        return data;
    }
    [mutating]
    public func store(path: PathState, weights: DirectWeights)->void
    {
        origin_pixel = path.origin;
        throughput = path.throughput;
        radiance = path.radiance;
        medium = path.medium;
        previous = path.previous;
        previous_pdf = path.previous_pdf;
        eta_scale = path.eta_scale;
        state = path.status | (path.depth << 2) | (path.dielectric_events << 9);
        sun.direction = path.direction;
        sun.pdf = weights.sun.x;
        sun.distance = weights.sun.y;
        lamp.pdf = weights.sun.z;
        lamp.direction = weights.lamp;
    }
    public func result()->PathState
    {
        return { origin_pixel,
                 sun.direction,
                 throughput,
                 radiance,
                 medium,
                 previous,
                 previous_pdf,
                 eta_scale,
                 (state >> 2) & 127u,
                 (state >> 9) & 127u,
                 state & 3u };
    }
    public func weights()->DirectWeights
    {
        return { float3(sun.pdf, sun.distance, lamp.pdf), lamp.direction };
    }
};
}
