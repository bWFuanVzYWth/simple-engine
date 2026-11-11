/// 实时一／二样本、分离 lane 与离线绝对样本的共同外层循环。
module pixel_transport;
import pt.scene;
import pt.hit_transport;
import pt.material_vertex;
import pt.integrator;
#ifndef PT_TRANSPORT_LANE
#define PT_TRANSPORT_LANE -1
#endif
namespace pt
{
/// 适配首面缓存、相机与导引；参数按路径读取，避免跨 callable 保留无关输入。
/// 状态初始化、样本域及失败传播由本模块维护。
public interface IPixelTransport
{
    uint sample_index(uint lane, bool realtime);
    float3 origin();
    float4 initial_medium();
    float2 start_path(inout TransportCall data, uint lane, bool realtime);
    float trace(inout TransportCall data, float2 film);
};
public struct PixelTransportInput
{
    public uint2 coordinate;
    /// 实时完整调度为 1 或 2；离线及分离 lane 不消费此值。
    public uint samples;
    public bool realtime;
};
public struct PixelTransportResult
{
    public float4 radiance;
    public float distance;
};
/// 离线累计计数和随机样本基数独立；实时 lane 使用连续的样本身份。
public uint pixel_sample_index(uint base, uint lane, bool realtime, uint accumulated,
                               uint batch_begin)
{
    return base + (realtime ? lane : (accumulated >= batch_begin ? accumulated - batch_begin : 0u));
}
/// previous 仅在分离 lane1 时消费；失败 lane0 不启动下一条路径。
/// 同一加法顺序和除数用于合并与分离调度，不创建嵌套调度。
public PixelTransportResult integrate_pixel<A : IPixelTransport>(A adapter, PixelTransportInput p,
                                                                 float4 previous)
{
    bool realtime = PT_TRANSPORT_LANE >= 0 || p.realtime;
#if PT_TRANSPORT_LANE >= 0
    const uint first = PT_TRANSPORT_LANE;
    const uint end = PT_TRANSPORT_LANE + 1;
    const uint paths = PT_TRANSPORT_LANE == 0 ? 1 : 2;
#else
    uint paths = realtime ? p.samples : 1u;
    uint first = 0;
    uint end = paths;
#endif
    TransportCall data = {};
#if PT_TRANSPORT_LANE == 1
    if (previous.w < 0)
        return { float4(previous.xyz * 0.5, previous.w), FAR };
    data.path.radiance = previous.xyz;
#endif
    float distance = FAR;
    for (uint lane = first; lane < end; lane++)
    {
        data.path.status = PATH_ACTIVE;
        data.path.depth = 0;
        data.path.dielectric_events = 0;
        data.path.previous = 0;
        data.path.previous_pdf = 0;
        data.pixel = p.coordinate;
        data.sample = adapter.sample_index(lane, realtime);
        data.path.origin = adapter.origin();
        data.path.throughput = 1;
        data.path.eta_scale = 1;
        data.path.medium = adapter.initial_medium();
        float2 film = adapter.start_path(data, lane, realtime);
        distance = adapter.trace(data, film);
        if (data.path.status == PATH_INVALID || data.path.status == PATH_DIELECTRIC_LIMIT)
            break;
    }
    return { float4(data.path.radiance / float(paths), data.path.status == PATH_INVALID ? -1
                                                       : data.path.status == PATH_DIELECTRIC_LIMIT
                                                           ? -3
                                                           : 0),
             distance };
}
}
