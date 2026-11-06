/// 完整材质顶点的closest-hit边界；几何和材质只在已经特化的命中程序中解码。
module material_vertex;
import pt.scene;
import pt.integrator;

namespace pt
{
/// 88B无损逻辑交接；输入像素/样本身份复用输出origin，不搬运Hit、纹素、法线或灯样本。
/// 输入state为depth[0..6]/dielectric_events[7..13]；输出为status[0..1]/depth/events。
/// 只在input之后调用unpack、store之后调用result；不能读取已被复用的输入。
public struct MaterialCall
{
    public float3 origin_pixel;
    public float3 direction;
    public float3 throughput;
    public float3 radiance;
    public float4 medium;
    public float3 previous;
    public float previous_pdf;
    public float eta_scale;
    public uint state;

    public static MaterialCall input(TransportCall data)
    {
        return { asfloat(uint3(data.pixel, data.sample)),
                 float3(0),
                 data.path.throughput,
                 data.path.radiance,
                 data.path.medium,
                 data.path.previous,
                 data.path.previous_pdf,
                 data.path.eta_scale,
                 data.path.depth | (data.path.dielectric_events << 7u) };
    }
    public TransportCall unpack(Hit hit, float3 incoming)
    {
        TransportCall data = {};
        data.hit = hit;
        data.pixel = asuint(origin_pixel.xy);
        data.sample = asuint(origin_pixel.z);
        data.path.direction = incoming;
        data.path.throughput = throughput;
        data.path.radiance = radiance;
        data.path.medium = medium;
        data.path.previous = previous;
        data.path.previous_pdf = previous_pdf;
        data.path.eta_scale = eta_scale;
        data.path.depth = state & 127u;
        data.path.dielectric_events = (state >> 7u) & 127u;
        return data;
    }
    [mutating]
    public void store(PathState path)
    {
        origin_pixel = path.origin;
        direction = path.direction;
        throughput = path.throughput;
        radiance = path.radiance;
        medium = path.medium;
        previous = path.previous;
        previous_pdf = path.previous_pdf;
        eta_scale = path.eta_scale;
        state = path.status | (path.depth << 2u) | (path.dielectric_events << 9u);
    }
    public PathState result()
    {
        return { origin_pixel,
                 direction,
                 throughput,
                 radiance,
                 medium,
                 previous,
                 previous_pdf,
                 eta_scale,
                 (state >> 2u) & 127u,
                 (state >> 9u) & 127u,
                 state & 3u };
    }
};

/// Flow和应用变体在资源世代确定；完整表面着色只看该材质需要的代码和资源。
/// 包含入射Beer、发光/MIS、选灯/支持域/阴影、BSDF、介质、RR和终止；不追踪下一主射线。
public void shade_material<let Flow : uint, Scene : IFlowScene, let LightMask : uint = 3>(
    Scene scene, inout MaterialCall call, Hit hit, float3 incoming, Settings settings)
{
    TransportCall data = call.unpack(hit, incoming);
    process_flow<Flow, Scene, LightMask>(scene, data, settings);
    call.store(data.path);
}
}
