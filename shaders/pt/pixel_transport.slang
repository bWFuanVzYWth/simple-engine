/// 实时双样本、分离 lane 与离线绝对样本的共同外层循环。
module pixel_transport;
import pt.scene;
import pt.hit_transport;
import pt.material_vertex;
import pt.integrator;
namespace pt {
/// 适配首面缓存、相机与导引；参数按路径读取，避免跨 callable 保留无关输入。
/// 状态初始化、样本域及失败传播由本模块维护。
public interface IPixelTransport {
    func sample_index(lane: uint)->uint;
    func origin()->float3;
    func initial_medium()->float4;
    func start_path(inout data: TransportCall, lane: uint)->float2;
    func trace(inout data: TransportCall, film: float2)->float;
};
public struct PixelTransportResult {
    public var radiance : float4;
    public var distance : float;
};

/// 新样本重置顶点身份，保留同像素已累计的radiance及适配器建立的射线方向。
public func begin_pixel_path(inout data: TransportCall)->void {
    data.path.status = PATH_ACTIVE;
    data.path.depth = 0;
    data.path.dielectric_events = 0;
    data.path.previous = 0;
    data.path.previous_pdf = 0;
}

/// 共享合并与分离lane的平均和粘滞错误编码；不重新调度路径。
public func pixel_result(radiance: float3, status: uint, paths: uint, distance: float)
    ->PixelTransportResult {
    return { float4(radiance / float(paths), status == PATH_INVALID            ? -1
                                             : status == PATH_DIELECTRIC_LIMIT ? -3
                                                                               : 0),
             distance };
}
/// 离线累计计数和随机样本基数独立；实时 lane 使用连续的样本身份。
public func pixel_sample_index(base: uint, lane: uint, realtime: bool, accumulated: uint,
                               batch_begin: uint)
    ->uint {
    return base + (realtime ? lane : (accumulated >= batch_begin ? accumulated - batch_begin : 0u));
}
/// 实时固定两条路径，lane=-1 时合并，0／1 分别选择其中一条。
/// 离线每次录制一条路径，lane 固定为 -1。
/// previous 仅在分离 lane1 时消费；失败 lane0 不启动下一条路径。
/// 同一加法顺序和除数用于合并与分离调度，不创建嵌套调度。
public func integrate_pixel<A : IPixelTransport, let realtime : bool, let selected_lane : int>(
    adapter: A, coordinate: uint2, previous: float4)
    ->PixelTransportResult {
    var adapter = adapter;
    let paths = realtime && selected_lane != 0 ? 2u : 1u;
    let first = selected_lane >= 0 ? uint(selected_lane) : 0u;
    let end = selected_lane >= 0 ? uint(selected_lane) + 1u : paths;
    var data : TransportCall = {};
    if (selected_lane == 1) {
        if (previous.w < 0)
            return { float4(previous.xyz * 0.5, previous.w), FAR };
        data.path.radiance = previous.xyz;
    }
    var distance : float = FAR;
    for (var lane: uint = first; lane < end; lane++) {
        begin_pixel_path(data);
        data.pixel = coordinate;
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
    return pixel_result(data.path.radiance, data.path.status, paths, distance);
}
}
