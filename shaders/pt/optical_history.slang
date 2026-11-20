/// 上一帧固定光路在像素中心采到的终点。每条路径用 24 位签名和两个 20 位局部坐标占 8 B。
module optical_history;
import sampling.sobol_bits;
namespace pt {
/// 由场景提供跨帧稳定的有向界面身份，按光路先后次序纳入签名。
public interface IOpticalPathKey {
    static func boundary(key: uint, hit: uint4, front: bool, reflection: bool, eta: float)->uint;
};
/// 只投影本帧几何的消费者无需访问上一帧的身份。
public struct NoOpticalPathKey : IOpticalPathKey {
    public static func boundary(key: uint, hit: uint4, front: bool, reflection: bool, eta: float)
        ->uint {
        return 0;
    }
};
/// 将 quad 身份及界面或终点类型混入已经过的光路签名。
public func optical_key(key: uint, quad: uint2, flags: uint)->uint {
    return sampling::sobol_bits::hash(sampling::sobol_bits::hash(key ^ quad.x) ^ quad.y ^ flags);
}
/// 打包非零路径签名及归一化终点坐标；全零记录表示这一像素没有可用的折射链。
public func optical_sample(key: uint, uv: float2)->uint2 {
    let xy = uint2(round(saturate(uv) * 1048575.0));
    let tag = max(key & 0xffffffu, 1u);
    return uint2((tag << 8) | (xy.x >> 12), (xy.x << 20) | xy.y);
}
/// 天空终点用立方体面的透视坐标。面号参与签名，插值不会跨越背向半球的参数接缝。
public func optical_sky(key: uint, direction: float3)->uint2 {
    let a = abs(direction);
    let axis = a.x >= a.y && a.x >= a.z ? 0u : (a.y >= a.z ? 1u : 2u);
    let uv = (axis == 0 ? direction.yz : (axis == 1 ? direction.zx : direction.xy)) / a[axis];
    let face = 2u * axis + uint(direction[axis] < 0);
    return optical_sample(optical_key(key, uint2(0), 4u + face), 0.5 + 0.5 * uv);
}
func coordinates(v: uint2)->float2 {
    return float2(((v.x & 255u) << 12) | (v.y >> 20), v.y & 0xfffffu);
}
/// 在旧帧同一路径的局部坐标中解逆映射。seed/result 以零为首像素中心。
/// 只接受有同路邻居覆盖的解；断面、不可逆映射、越界或迭代未收敛均拒绝历史。
public func previous_optical_pixel<let reflected : bool>(RWStructuredBuffer<uint4> records,
                                                         uint2 extent, uint2 current, float2 seed,
                                                         out float2 result)
    ->bool {
    result = seed;
    let key = current.x >> 8;
    if (key == 0 || any(extent < 2) || !all(isfinite(seed)))
        return false;
    // 切平面的初值可落在屏外；只由实际旧采样的覆盖判断最终解是否可见。
    result = clamp(seed, float2(0), float2(extent - 1));
    let target = coordinates(current);
    for (var iteration = 0u; iteration < 4u; ++iteration) {
        if (any(result < 0) || any(result > float2(extent - 1)))
            return false;
        let cell = min(uint2(floor(result)), extent - 2);
        let base = cell.y * extent.x + cell.x;
        var xy : float2[4];
        var mask = 0u;
        [unroll]
        for (var i = 0u; i < 4u; ++i) {
            let pair = records[base + (i & 1u) + (i >> 1u) * extent.x];
            let record = reflected ? pair.zw : pair.xy;
            mask |= uint((record.x >> 8) == key) << i;
            xy[i] = coordinates(record);
        }
        if (countbits(mask) < 3)
            return false;
        let origin = mask == 15u ? 0u : (uint(firstbitlow((~mask) & 15u)) ^ 3u);
        let corner = float2(origin & 1u, origin >> 1u);
        let sign = 1 - 2 * corner;
        let anchor = mask == 15u ? result - float2(cell) : corner;
        var dx = (xy[origin ^ 1u] - xy[origin]) * sign.x;
        var dy = (xy[origin ^ 2u] - xy[origin]) * sign.y;
        var mapped = xy[origin];
        if (mask == 15u) {
            let mixed = (xy[3] - xy[2]) - dx;
            mapped += dx * anchor.x + dy * anchor.y + mixed * (anchor.x * anchor.y);
            dx += mixed * anchor.y;
            dy += mixed * anchor.x;
        }
        let det = dx.x * dy.y - dx.y * dy.x;
        if (abs(det) < max(1.0, 1e-4 * length(dx) * length(dy)))
            return false;
        // 当前和旧帧插值各有半个坐标量化单位的不确定度。通过逆雅可比换回输入像素，
        // 在终点局部坐标过于粗糙或掠射退化处放弃复用，给迭代残差保留亚像素余量。
        let uncertainty = float2(abs(dy.x) + abs(dy.y), abs(dx.x) + abs(dx.y));
        if (dot(uncertainty, uncertainty) > (0.75 * det) * (0.75 * det))
            return false;
        let uv = target - mapped;
        let offset = float2(uv.x * dy.y - uv.y * dy.x, dx.x * uv.y - dx.y * uv.x) / det;
        let local = anchor + offset;
        let next = float2(cell) + local;
        // 四点时反求双线性映射；三点时只在已知三角形内插值。
        // 静止镜头的解常在单元角上。允许浮点舍入越边千分之一像素，避免把同一采样判为显露。
        let edge_epsilon = 0.001;
        if (all(local >= -edge_epsilon) && all(local <= 1 + edge_epsilon) &&
            (mask == 15u ? all(abs(offset) < 0.02) : dot(offset, sign) <= 1 + edge_epsilon)) {
            result = float2(cell) + saturate(local);
            return true;
        }
        if (all(abs(next - result) < 0.01))
            return false;
        result = next;
    }
    return false;
}
}
