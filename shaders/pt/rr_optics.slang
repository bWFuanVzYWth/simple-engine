/// RR 采样历史在输入像素格上的反求。两条光路独立匹配，任一路失效通知 SDK 显露。
module rr_optics;
import pt.optical_history;
import pt.reconstruction;
import pt.guide_output;
namespace pt {
/// x 为零时 y 区分普通投影与本帧应拒绝的投影，二者都不能充当旧光路样本。
public func rr_optical_record(record: uint2, flags: uint)->uint2 {
    let invalid = ((flags & (GUIDE_INVALID | GUIDE_SURFACE_PROXY)) != 0) |
                  ((record.x == 0) & ((flags & GUIDE_UNPROJECTABLE) != 0));
    return invalid ? uint2(0, 1) : record;
}

func previous_keys(RWStructuredBuffer<uint2> previous, uint2 extent, float2 raster)->uint2 {
    if (!all(raster >= 0) || !all(raster < float2(extent)))
        return 0;
    let xy = uint2(raster);
    return previous[xy.y * extent.x + xy.x] >> 8;
}

func match_rr_lane<let reflected : bool>(RWStructuredBuffer<uint2> previous, uint2 extent,
                                         uint2 record, float2 origin, inout float2 motion)
    ->bool {
    var matched : float2;
    if (!previous_optical_pixel<reflected>(previous, extent, record, origin + motion, matched))
        return false;
    motion = matched - origin;
    return true;
}

/// 与 HDR 打包同一 compute 调度。只写获得亚像素修正的 motion，普通投影沿用导引输出。
public func resolve_rr_optics(RWStructuredBuffer<uint2> current, RWStructuredBuffer<uint2> previous,
                              RWTexture2D<float2> motion, RWTexture2D<float2> specular_motion,
                              RWTexture2D<float> disocclusion, uint2 pixel, uint2 extent,
                              float2 jitter_delta, bool history)
    ->void {
    if (!history) {
        disocclusion[pixel] = 0;
        return;
    }
    let index = pixel.y * extent.x + pixel.x;
    let tag = current[index];
    let active = tag >= 256u;
    let origin = float2(pixel) + jitter_delta;
    var inside = motion[pixel];
    var outside = specular_motion[pixel];
    var reject = any(tag == 1u);
    var keys : uint2 = 0;
    if (!active.x) {
        keys = previous_keys(previous, extent, origin + 0.5 + inside);
        reject = reject || keys.x != 0;
    }
    if (!active.y) {
        if (active.x || any(inside != outside))
            keys.y = previous_keys(previous, extent, origin + 0.5 + outside).y;
        reject = reject || keys.y != 0;
    }
    if (!any(active)) {
        disocclusion[pixel] = float(reject);
        return;
    }
    let low = current[extent.x * extent.y + index];
    if (active.x) {
        if (match_rr_lane<false>(previous, extent, uint2(tag.x, low.x), origin, inside))
            motion[pixel] = inside;
        else
            reject = true;
    }
    if (active.y) {
        if (match_rr_lane<true>(previous, extent, uint2(tag.y, low.y), origin, outside))
            specular_motion[pixel] = outside;
        else
            reject = true;
    }
    disocclusion[pixel] = float(reject);
}

/// 时域导引和线性 HDR 都已就绪时写入 RR 的输入。可由合成调度或独立打包入口执行。
public struct RrInputs {
    public var noisy : RWTexture2D<float4>;
    public var even : RWStructuredBuffer<uint2>;
    public var odd : RWStructuredBuffer<uint2>;
    public var motion : RWTexture2D<float2>;
    public var specular_motion : RWTexture2D<float2>;
    public var disocclusion : RWTexture2D<float>;
    public func store(hdr: float3, pixel: uint2, extent: uint2, slot: uint, jitter_delta: float2,
                      history: bool)
        ->void {
        noisy[pixel] = reconstruction_input(hdr);
        if ((slot & 1u) == 0)
            resolve_rr_optics(even, odd, motion, specular_motion, disocclusion, pixel, extent,
                              jitter_delta, history);
        else
            resolve_rr_optics(odd, even, motion, specular_motion, disocclusion, pixel, extent,
                              jitter_delta, history);
    }
}
}
