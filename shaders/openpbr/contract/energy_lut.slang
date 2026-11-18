// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 能量表查询能力；不拥有纹理或设备资源。
module energy_lut;
namespace openpbr {
/// 原作者的标定表；lookup固定采用normalized linear clamp与LOD0，查询次数由数值核决定。
public interface IEnergyLut {
    public func lookup(uvw: float3)->float4;
}
}
