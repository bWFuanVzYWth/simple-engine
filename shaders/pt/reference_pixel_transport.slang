/// 显式参考调度；保留旧动态模式，用于固定生产入口的行为对照。
module reference_pixel_transport;
import pt.pixel_transport;
import pt.scene;
import pt.hit_transport;
import pt.material_vertex;
import pt.integrator;
namespace pt
{
/// 适配首面缓存、相机与导引；参数按路径读取，避免跨 callable 保留无关输入。
/// 状态初始化、样本域及失败传播由本模块维护。
public interface IReferencePixelTransport
{
    func sample_index(lane: uint, realtime: bool)->uint;
    func origin()->float3;
    func initial_medium()->float4;
    func start_path(inout data: TransportCall, lane: uint, realtime: bool)->float2;
    func trace(inout data: TransportCall, film: float2)->float;
};
public struct ReferencePixelTransportInput
{
    public var coordinate : uint2;
    /// 实时完整调度为 1 或 2；离线及分离 lane 不消费此值。
    public var samples : uint;
    public var realtime : bool;
};
/// Lane 为 -1 时由输入选择实时／离线调度，0／1 选择分离 lane。
/// previous 仅在分离 lane1 时消费；失败 lane0 不启动下一条路径。
/// 同一加法顺序和除数用于合并与分离调度，不创建嵌套调度。
public func integrate_reference_pixel<A : IReferencePixelTransport, let Lane : int>(
    adapter: A, p: ReferencePixelTransportInput, previous: float4)
    ->PixelTransportResult
{
    var adapter = adapter;
    let realtime = Lane >= 0 || p.realtime;
    let paths = Lane >= 0 ? (Lane == 0 ? 1u : 2u) : (realtime ? p.samples : 1u);
    let first = Lane >= 0 ? uint(Lane) : 0u;
    let end = Lane >= 0 ? uint(Lane) + 1u : paths;
    var data : TransportCall = {};
    if (Lane == 1)
    {
        if (previous.w < 0)
            return { float4(previous.xyz * 0.5, previous.w), FAR };
        data.path.radiance = previous.xyz;
    }
    var distance : float = FAR;
    for (var lane: uint = first; lane < end; lane++)
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
        var film : float2 = adapter.start_path(data, lane, realtime);
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
