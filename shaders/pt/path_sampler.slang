/// PT 的固定消费 profile：保护灯和 BSDF 的组内半区投影，接受 selector/event 两点退化。
/// 引擎原语的全部缺陷见 pt.binary_sobol；本模块不承诺通用高维质量或实测 RR 收益。
module path_sampler;

import pt.binary_sobol;
import pt.sampling;

namespace pt
{
/// domain 分配不随分支或已抽到的事件改变；跳过消费者不推进其他域。
/// 同一交点太阳／局部灯是加性 NEE，故优先保护各组内部；跨组与后续路径仍可能耦合。
/// selector/event 仍控制高能量路径，并非无害坐标；两点阈值误差是明确接受的代价。
/// 固定 profile 不按材质能力另选映射；方法支持depth<=64；depth=64只供末顶点NEE。RR 保持普通一维，
/// 但这不保证 RR 与刚抽到的 BSDF throughput 独立，也不消除跨 bounce 相关。
public struct PathSampler
{
    internal var core : pt::BinarySobolS8;

    /// 相机像素内偏移，不与 BSDF／光源域合组。
    public func film()->float2
    {
        return core.sample_2d(0);
    }
    /// 独立太阳盘的二维采样；depth<=64。
    public func sun(depth: uint)->float2
    {
        return core.sample_2d(1 + 8 * depth);
    }

    /// xy 灯面 UV、z BVH 选择；牺牲 z 两点分层，保护 selector×UV 四点粗投影。
    public func light(depth: uint)->float3
    {
        let domain : uint = 1 + 8 * depth;
        return core.sample_3d(domain + 2, domain + 1);
    }

    /// xy OpenPBR 方向、z lobe/Fresnel；同样牺牲事件的两点分层，不改方向序列。
    public func bsdf(depth: uint)->float3
    {
        let domain : uint = 1 + 8 * depth;
        return core.sample_3d(domain + 3, domain + 4);
    }

    /// 存活阈值的一维采样；不承担 deferred 两点代价，不保证与 BSDF 独立。
    public func roulette(depth: uint)->float
    {
        return core.sample_1d(1 + 8 * depth + 5);
    }
};

/// 绝对样本分成 256 点 epoch；分批提交相同样本身份，跨 epoch 不合并宣称 net 保证。
public func path_sampler(pixel: uint2, absolute_sample: uint, r: uint, seed: uint)->PathSampler
{
    var result : PathSampler;
    let scramble = pt::pcg(seed ^ pt::pcg(absolute_sample >> 8));
    result.core = pt::binary_sobol_s8(pixel, absolute_sample & 255u, r, scramble);
    return result;
}
}
