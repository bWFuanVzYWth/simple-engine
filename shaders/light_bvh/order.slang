/// 两种灯树的 Morton 分段编码与确定性全序。
module "light_bvh/order.slang";
namespace light_bvh {
func spread10(v: uint)->uint {
    var v = v;
    v &= 1023u;
    v = (v | (v << 16u)) & 0x30000ffu;
    v = (v | (v << 8u)) & 0x300f00fu;
    v = (v | (v << 4u)) & 0x30c30c3u;
    return (v | (v << 2u)) & 0x9249249u;
}
/// 交错三个坐标各自的低十位，输出的第 3i、3i+1、3i+2 位依次来自 x、y、z 的第 i 位。
public func morton10(v: uint3)->uint {
    return spread10(v.x) | (spread10(v.y) << 1u) | (spread10(v.z) << 2u);
}
/// 按 x、y、z、w 的无符号字典序比较，最低优先级字段可携带稳定叶号。
public func less_key(a: uint4, b: uint4)->bool {
    return a.x < b.x ||
           (a.x == b.x && (a.y < b.y || (a.y == b.y && (a.z < b.z || (a.z == b.z && a.w < b.w)))));
}
}
