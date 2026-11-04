// 来源：Sky Tracer 9e9430d；见 crates/atmosphere/data/README.md。
module parameters;
namespace atmosphere
{
/// 固定介质、四波长及观察者参数，按源顺序排列的 400 B；数组均为连续 float4。
/// 距离为 km；局部 +Y 为径向上方，太阳方位 atan2(x,z)，仰角 asin(y)。
/// 数据偏移与维度须来自同一物理输入；API 不支持混合不同求解版本的 LUT。
public struct Params
{
    /// xy 输出大小，z 积分步数，w 位 0 为 MS、位 1 为太阳显示、位 2 为光谱诊断。
    public uint4 size_steps;
    /// yaw、pitch、垂直 FOV（弧度）与观察者海拔 km。
    public float4 view;
    /// 太阳方位/仰角/半径（弧度）及行星底部半径 km。
    public float4 sun;
    /// source 高度数、太阳状态数、optical 高度数、optical 角度数。
    public uint4 dims;
    /// 系数、保留位、剖面、相函数在 float4 物理输入中的偏移。
    public uint4 offsets0;
    /// 相函数质量偏移、低空高度数、剖面长度、保留位。
    public uint4 offsets1;
    /// 大气厚度 km、地表反照率、有限线段 km（0 为边界全程）、地平线采样幂。
    public float4 medium;
    /// 450/510/580/650 nm 的太阳辐照谱密度。
    public float4 solar;
    /// 四波长到线性 Rec.2020 的拟合系数，w 不使用。
    public float4 rgb[4];
    /// 入射方向数、相位数、cone 数、迭代号（首轮 1 跳过先前 volume source）。
    public uint4 solve;
    /// 状态、积分角节点、相位节点的偏移及 optical 积分步数。
    public uint4 extra;
    /// 坐标模式位及 solar/optical/cone 节点偏移；balanced 模式为 63。
    public uint4 mapping;
    /// SkyView 宽高、低天空行数、全部天空行数，其余行为地面 chart。
    public uint4 sky;
    /// 六段 optical 高度映射系数。
    public float4 optical_segments[6];
    /// 拟合 phase 权重与 cone warp。
    public float4 angular_mapping;
    /// 太阳重要性宽度、路径高度尺度 km、高太阳权重及保留位。
    public float4 importance;
    /// 当前高度批的首项/数量及 optical 行更新范围。
    public uint4 batch;
};

}
