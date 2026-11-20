/// 每条实际主路径的两个波瓣、世界空间方向矩和首次交点。每像素两条 lane。
module nrd_signal;

public struct SignalLane {
    public var diffuse : uint2;
    public var specular : uint2;
    /// xyz 为方向矩，w 同时保留首表面关于采样太阳方向的几何余弦绝对值。
    public var diffuse_direction : uint2;
    /// xyz 为方向矩，w 为该路径实际采到的首面散射事件。
    public var specular_direction : uint2;
    public var sun : uint2;
};
public func pack_signal(v: float4)->uint2 {
    let h = f32tof16(clamp(v, -65504, 65504));
    return h.xz | (h.yw << 16);
}
public func unpack_signal(h: uint2)->float4 {
    return f16tof32(uint4(h.x & 65535, h.x >> 16, h.y & 65535, h.y >> 16));
}
public func brightness(rgb: float3)->float {
    return dot(rgb, float3(0.2126, 0.7152, 0.0722));
}

/// 有符号八面体法线，供高光导引以两个 FP16 分量往返。
public func oct_normal(n: float3)->float2 {
    let p = n.xy / dot(abs(n), float3(1));
    return n.z >= 0 ? p : (1 - abs(p.yx)) * select(p >= 0, 1.0, -1.0);
}
public func from_oct_normal(p: float2)->float3 {
    var n = float3(p, 1 - abs(p.x) - abs(p.y));
    n.xy += select(n.xy >= 0, -1.0, 1.0) * saturate(-n.z);
    return normalize(n);
}
