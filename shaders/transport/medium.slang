// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only

/// 均匀介质的光谱近似。
module medium;

namespace transport {
/// 均匀介质 Beer 透射；extinction 非负、distance 非负且单位互逆，不包含散射源。
public func segment_transmittance(extinction: float3, distance: float)->float3 {
    return exp(-extinction * distance);
}
}
