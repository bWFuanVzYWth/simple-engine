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
/// 将同一四边形的 ABC、ACD 两组三角形重心坐标映到连续的单位方形。
public func optical_quad(key: uint, primitive: uint, bary: float2)->uint2 {
    let uv =
        (primitive & 1u) == 0 ? float2(bary.x + bary.y, bary.y) : float2(bary.x, bary.x + bary.y);
    return optical_sample(key, uv);
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
/// 首列存两路的签名及高位坐标。零为普通投影，一为失效。只有有效光路需要写第二列。
public func store_optical_pair(RWStructuredBuffer<uint2> grid, uint index, uint count, uint4 pair)
    ->void {
    let keys =
        uint2(pair.x == 0 ? uint(pair.y != 0) : pair.x, pair.z == 0 ? uint(pair.w != 0) : pair.z);
    grid[index] = keys;
    if (any(keys >= 256u))
        grid[count + index] = pair.yw;
}
/// 在旧帧同一路径的局部坐标中解逆映射。seed/result 以零为首像素中心。
/// 只接受有同路邻居覆盖的解；断面、不可逆映射、越界或迭代未收敛均拒绝历史。
public func previous_optical_pixel<let reflected : bool>(RWStructuredBuffer<uint2> records,
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
            let index = base + (i & 1u) + (i >> 1u) * extent.x;
            let tag = records[index][uint(reflected)];
            let same = (tag >> 8) == key;
            mask |= uint(same) << i;
            xy[i] = 0;
            if (same) {
                let low = records[index + extent.x * extent.y][uint(reflected)];
                xy[i] = coordinates(uint2(tag, low)) - target;
            }
        }
        if (countbits(mask) < 3)
            return false;
        var anchor = result - float2(cell);
        var dx = xy[1] - xy[0];
        var dy = xy[2] - xy[0];
        var mapped = xy[0];
        var mixed = float2(0);
        if (mask == 15u) {
            mixed = (xy[3] - xy[2]) - dx;
            mapped += dx * anchor.x + dy * anchor.y + mixed * (anchor.x * anchor.y);
            dx += mixed * anchor.y;
            dy += mixed * anchor.x;
        } else {
            // 三个有效角总有一行一列完整。固定下标避免把四个寄存器变成动态寻址数组。
            let right = (mask & 5u) != 5u;
            let bottom = (mask & 3u) != 3u;
            anchor = float2(right, bottom);
            dx = bottom ? xy[3] - xy[2] : dx;
            dy = right ? xy[3] - xy[1] : dy;
            mapped = bottom ? (right ? xy[3] : xy[2]) : (right ? xy[1] : xy[0]);
        }
        let det = dx.x * dy.y - dx.y * dy.x;
        if (det * det < max(1., 1e-8 * dot(dx, dx) * dot(dy, dy)))
            return false;
        // 当前和旧帧插值各有半个坐标量化单位的不确定度。通过逆雅可比换回输入像素，
        // 在终点局部坐标过于粗糙或掠射退化处放弃复用，给迭代残差保留亚像素余量。
        let uncertainty = float2(abs(dy.x) + abs(dy.y), abs(dx.x) + abs(dx.y));
        if (dot(uncertainty, uncertainty) > (0.75 * det) * (0.75 * det))
            return false;
        let uv = -mapped;
        let offset = float2(uv.x * dy.y - uv.y * dy.x, dx.x * uv.y - dx.y * uv.x) / det;
        let local = anchor + offset;
        let next = float2(cell) + local;
        // 四点时反求双线性映射；三点时只在已知三角形内插值。
        // 静止镜头的解常在单元角上。允许浮点舍入越边千分之一像素，避免把同一采样判为显露。
        let edge_epsilon = 0.001;
        // 双线性插值在 Newton 步后的精确剩余项为 mixed * offset.x * offset.y。
        // 仿射单元一次便收敛；用解处的逆雅可比检查像素误差，无须重复读取同一四点。
        if (all(local >= -edge_epsilon) && all(local <= 1 + edge_epsilon)) {
            var covered = mask == 15u || dot(offset, 1 - 2 * anchor) <= 1 + edge_epsilon;
            if (mask == 15u && any(mixed != 0)) {
                let jx = dx + mixed * offset.y;
                let jy = dy + mixed * offset.x;
                let jacobian = abs(jx.x * jy.y - jx.y * jy.x);
                let bound = float2(abs(jy.x) + abs(jy.y), abs(jx.x) + abs(jx.y));
                let residual = mixed * (offset.x * offset.y);
                let correction = float2(residual.x * jy.y - residual.y * jy.x,
                                        jx.x * residual.y - jx.y * residual.x);
                covered = jacobian * jacobian >= max(1., 1e-8 * dot(jx, jx) * dot(jy, jy)) &&
                          dot(bound, bound) <= (0.75 * jacobian) * (0.75 * jacobian) &&
                          all(abs(correction) < 0.01 * jacobian);
            }
            if (covered) {
                result = float2(cell) + saturate(local);
                return true;
            }
        }
        if (all(abs(next - result) < 0.01))
            return false;
        result = next;
    }
    return false;
}
}
