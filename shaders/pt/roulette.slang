// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 无资源的 Russian roulette 与折射率补偿，不决定反弹预算、随机域或开始顶点。
/// throughput 须有限非负、eta_scale 有限正；调用方维持乘除可表示且正 metric 不下溢。
/// 不校验或修复不支持的数值输入，不添加存活率上下护栏。
module roulette;

namespace pt
{
/// relative_eta=eta_t/eta_i，透射更新平方补偿，反射/TIR 不变。
/// eta_scale 和 relative_eta 须有限正，更新结果须可表示。
public float eta_scale_after_scatter(float eta_scale, float relative_eta, bool transmission)
{
    return transmission ? eta_scale * relative_eta * relative_eta : eta_scale;
}

/// 从已完成散射数决定应用选定的起点；未发生散射的透明孔不应计数。
public bool roulette_applies(uint completed_scatters, uint minimum_completed_scatters)
{
    return completed_scatters >= minimum_completed_scatters;
}

/// p=clamp(maxRGB(throughput)*eta_scale,0,1)。p=0 结束，p=1 原样继续；两者均不取随机数。
/// 仅 0<p<1 时取独立均匀 u，以 u<p 决定存活，再调用 roulette_reweight。
public float roulette_probability(float3 throughput, float eta_scale)
{
    return clamp(max(throughput.x, max(throughput.y, throughput.z)) * eta_scale, 0.0, 1.0);
}

/// 仅用于 0<p<1 的存活事件；条件贡献除以 p，保持期望。p=1 路径不需要调用。
public float3 roulette_reweight(float3 throughput, float probability)
{
    return throughput / probability;
}
}
