/// 每条实际主路径的两个波瓣、世界空间方向矩和首次交点。每像素两条 lane。
module nrd_signal;

public struct SignalLane {
    public var diffuse : uint2;
    public var specular : uint2;
    public var diffuse_direction : uint2;
    public var specular_direction : uint2;
    public var sun : uint2;
    /// RGB 是独立合成的发光和互补光路，w 为几何表面与采样太阳方向夹角余弦的绝对值。
    public var emission : uint2;
    /// 追踪期间为漫反射比例与事件。交付时 lane 0 为虚拟观察方向，lane 1 为光学前缀 RGB 吞吐；
    /// 两者的 w 均为单位 view-Z 对应的像素高度。
    public var fraction : uint2;
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
