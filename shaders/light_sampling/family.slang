/// 局部灯与环境灯的条件采样域。
module family;

namespace light_sampling {
/// 两个族分别独立采一次，返回条件随机数；PDF不乘二选一概率，缺失族贡献为0。
/// 输入z在[0,1)，下半区为局部灯，上半区为环境灯；调用方不能按活动族重新分配。
public func local_light_family(inout u: float3)->bool {
    let local = u.z < 0.5;
    u.z = local ? 2.0 * u.z : 2.0 * u.z - 1.0;
    return local;
}
}
