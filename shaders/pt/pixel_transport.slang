/// 实时一／二样本、分离 lane 与离线绝对样本的共同外层循环。
module pixel_transport;
import pt.scene;
import pt.hit_transport;
import pt.material_vertex;
import pt.integrator;
namespace pt
{
/// 适配首面缓存、相机与导引；参数按路径读取，避免跨 callable 保留无关输入。
/// 状态初始化、样本域及失败传播由本模块维护。
public interface IPixelTransport
{
    func sample_index(lane: uint)->uint;
    func origin()->float3;
    func initial_medium()->float4;
    func start_path(inout data: TransportCall, lane: uint)->float2;
    func trace(inout data: TransportCall, film: float2)->float;
};
public struct PixelTransportInput
{
    public var coordinate : uint2;
};
public struct PixelTransportResult
{
    public var radiance : float4;
    public var distance : float;
};
/// 离线累计计数和随机样本基数独立；实时 lane 使用连续的样本身份。
public func pixel_sample_index(base: uint, lane: uint, realtime: bool, accumulated: uint,
                               batch_begin: uint)
    ->uint
{
    return base + (realtime ? lane : (accumulated >= batch_begin ? accumulated - batch_begin : 0u));
}
/// 实时1／2样本与离线由受控入口固定；Lane为-1时合并，0／1选择分离lane。
/// 实时Samples须为1或2，分离Lane须小于Samples；离线固定Samples=1、Lane=-1。
/// previous 仅在分离 lane1 时消费；失败 lane0 不启动下一条路径。
/// 同一加法顺序和除数用于合并与分离调度，不创建嵌套调度。
public func integrate_pixel<A : IPixelTransport, let Realtime : bool, let Samples : uint,
                            let Lane : int>(adapter: A, p: PixelTransportInput, previous: float4)
    ->PixelTransportResult
{
    var adapter = adapter;
    let paths = Lane >= 0 ? (Lane == 0 ? 1u : 2u) : (Realtime ? Samples : 1u);
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
        data.sample = adapter.sample_index(lane);
        data.path.origin = adapter.origin();
        data.path.throughput = 1;
        data.path.eta_scale = 1;
        data.path.medium = adapter.initial_medium();
        let film = adapter.start_path(data, lane);
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
