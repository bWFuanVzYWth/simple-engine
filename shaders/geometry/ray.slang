// SPDX-FileCopyrightText: Copyright (c) 2023 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: BSD-3-Clause
// 许可证：../../LICENSES/NVIDIA-self-intersection-avoidance.txt
// 移植自：https://github.com/NVIDIA/self-intersection-avoidance/tree/5a5c45bc61aca5d095032860058481e0497350e0

/// 三角形表面重建与自相交规避，不执行场景求交或遮挡判断。
/// 输入和中间运算须处于 float32 有效范围，三角形不得发生浮点退化；保留 precise、FMA 和运算顺序。
/// 误差界包含三角形重建、硬件求交和双向实例变换；求交系数面向 NVIDIA RTX，其他 GPU 需验证。
/// 极掠射仍可能受方向舍入影响，不能视为任意场景的统一保证。
/// 上游实现与 BSD-3-Clause
/// 许可见文件头；分析：https://developer.nvidia.com/blog/solving-self-intersection-artifacts-in-directx-raytracing/
module ray;

namespace geometry
{
/// p(t) = origin + t * direction；方向可不归一化，只有单位方向的 t 才是距离。
public struct Ray
{
    /// 射线起点，与 direction 使用同一坐标系。
    public float3 origin;
    /// 沿 direction 的参数下界，不是独立的距离单位。
    public float t_min;
    /// 参数方向，长度决定 t 到距离的换算。
    public float3 direction;
    /// 沿 direction 的参数上界；调用方维持有效区间。
    public float t_max;
};

/// 可直接生成散射射线的世界空间表面点与误差偏移。
public struct SurfacePoint
{
    /// 尚未按出射方向偏移的世界位置。
    public float3 position;
    /// 单位几何法线；三角形绕序经逆转置变换后的朝向。
    public float3 normal;
    /// 沿单位法线的非负世界距离，不是射线 t 参数。
    public float offset;
};

/// 可供多条连接复用的目标端点；缓存与连接方向无关的误差项。
public struct TriangleEndpoint
{
    /// 未偏移的世界位置。
    public float3 position;
    /// 单位世界几何法线，保留物体空间三角形绕序。
    public float3 normal;
    /// 不含 world-to-object 线性位置误差的法线偏移，连接时再补齐。
    public float base_offset;
    /// 与 abs(origin)+abs(未偏移方向) 点乘，补充目标端的遍历误差距离。
    public float3 traversal_weight;
};

struct TriangleData
{
    float3 position;
    float3 normal;
    float3 error;
};

static const float c0 = 5.9604644775390625E-8;
static const float c1 = 1.788139769587360206060111522674560546875E-7;
static const float c2 = 1.19209317972490680404007434844970703125E-7;

TriangleData reconstruct_triangle(float3 v0, float3 v1, float3 v2, float2 bary)
{
    precise float3 edge1 = v1 - v0;
    precise float3 edge2 = v2 - v0;
    precise float3 position = v0 + fma(float3(bary.x), edge1, bary.y * edge2);
    float3 extent3 = abs(edge1) + abs(edge2) + abs(edge1 - edge2);
    float extent = max(max(extent3.x, extent3.y), extent3.z);
    TriangleData point;
    point.position = position;
    point.normal = cross(edge1, edge2);
    point.error = fma(float3(c0), abs(v0), float3(c1 * extent));
    return point;
}

/// 从物体空间三角形重建可复用世界端点；bary 对应 (v1-v0, v2-v0)，须非负且和不大于 1。
/// 两个仿射 float3x4 变换须互逆、有限、非奇异且与实际实例一致；其余数值边界见模块说明。
public TriangleEndpoint triangle_endpoint(float3 v0, float3 v1, float3 v2, float2 bary,
                                          float3x4 object_to_world, float3x4 world_to_object)
{
    TriangleData object = reconstruct_triangle(v0, v1, v2, bary);
    precise float3 world_position;
    world_position.x = object_to_world[0][3] + fma(object_to_world[0][0], object.position.x,
                                                   fma(object_to_world[0][1], object.position.y,
                                                       object_to_world[0][2] * object.position.z));
    world_position.y = object_to_world[1][3] + fma(object_to_world[1][0], object.position.x,
                                                   fma(object_to_world[1][1], object.position.y,
                                                       object_to_world[1][2] * object.position.z));
    world_position.z = object_to_world[2][3] + fma(object_to_world[2][0], object.position.x,
                                                   fma(object_to_world[2][1], object.position.y,
                                                       object_to_world[2][2] * object.position.z));
    float3 world_normal = mul(transpose((float3x3)world_to_object), object.normal);
    float world_scale = rsqrt(dot(world_normal, world_normal));
    world_normal *= world_scale;

    float3 translation =
        float3(object_to_world[0][3], object_to_world[1][3], object_to_world[2][3]);
    float3x3 object_absolute = float3x3(abs(object_to_world[0].xyz), abs(object_to_world[1].xyz),
                                        abs(object_to_world[2].xyz));
    float3x3 inverse_absolute = float3x3(abs(world_to_object[0].xyz), abs(world_to_object[1].xyz),
                                         abs(world_to_object[2].xyz));
    float3 world_error =
        fma(float3(c1), mul(object_absolute, abs(object.position)), c2 * abs(translation));
    float3 inverse_translation =
        float3(world_to_object[0][3], world_to_object[1][3], world_to_object[2][3]);
    float3 object_error = fma(float3(c2), abs(inverse_translation), object.error);
    TriangleEndpoint point;
    point.position = world_position;
    point.normal = world_normal;
    point.base_offset = fma(world_scale, dot(object_error, abs(object.normal)),
                            dot(world_error, abs(world_normal)));
    point.traversal_weight =
        (c2 * world_scale) * mul(transpose(inverse_absolute), abs(object.normal));
    return point;
}

/// 无实例变换的重载，输入即世界坐标；保留恒等变换误差预算，其余三角形条件同变换重载。
public TriangleEndpoint triangle_endpoint(float3 v0, float3 v1, float3 v2, float2 bary)
{
    TriangleData object = reconstruct_triangle(v0, v1, v2, bary);
    float scale = rsqrt(dot(object.normal, object.normal));
    TriangleEndpoint point;
    point.position = object.position;
    point.normal = object.normal * scale;
    point.base_offset = fma(scale, dot(object.error, abs(object.normal)),
                            dot(c1 * abs(object.position), abs(point.normal)));
    point.traversal_weight = c2 * abs(point.normal);
    return point;
}

/// 将缓存端点转换为完整当前位置的误差偏移；endpoint须满足triangle_endpoint契约。
public SurfacePoint surface_point(TriangleEndpoint endpoint)
{
    SurfacePoint point;
    point.position = endpoint.position;
    point.normal = endpoint.normal;
    point.offset = endpoint.base_offset + dot(endpoint.traversal_weight, abs(endpoint.position));
    return point;
}

/// 使用与实际实例一致的互逆仿射变换重建世界表面点，包含当前位置的完整法线偏移距离。
/// bary、非退化和 float32 范围条件同 triangle_endpoint；法线按物体绕序经逆转置变换。
public SurfacePoint triangle_point(float3 v0, float3 v1, float3 v2, float2 bary,
                                   float3x4 object_to_world, float3x4 world_to_object)
{
    return surface_point(triangle_endpoint(v0, v1, v2, bary, object_to_world, world_to_object));
}

/// 无实例变换的世界坐标重载，保留恒等变换误差；重心与数值边界同 triangle_endpoint。
public SurfacePoint triangle_point(float3 v0, float3 v1, float3 v2, float2 bary)
{
    return surface_point(triangle_endpoint(v0, v1, v2, bary));
}

/// 沿几何法线偏移世界位置；direction 只选前／后侧，不要求单位长度，切向时选前侧。
/// point 须携带单位几何法线及有效非负 offset；不修改传入的表面点。
public float3 offset_origin(SurfacePoint point, float3 direction)
{
    float offset = dot(point.normal, direction) < 0.0 ? -point.offset : point.offset;
    precise float3 origin = fma(float3(offset), point.normal, point.position);
    return origin;
}

/// 从 offset_origin 选定侧生成散射射线，t_min=0，原样保留 direction 与 t_max。
/// 不归一化方向或换算上界；仅单位方向时 t_max 是世界距离，调用方维持有效区间。
public Ray spawn_ray(SurfacePoint point, float3 direction, float t_max)
{
    Ray ray;
    ray.origin = offset_origin(point, direction);
    ray.t_min = 0.0;
    ray.direction = direction;
    ray.t_max = t_max;
    return ray;
}

/// 构造 source 到 target 的连接段，使用非单位方向与 [0, nextDown(1)]，避免归一化和距离换算。
/// 目标端误差使用 abs(origin)+abs(未偏移方向)，沿法线以 FMA 修正方向，避免重建端点的新误差。
/// false 表示端点重合或偏移后无法形成正向非空段，此时不能追踪输出；不表示几何遮挡。
/// 连接射线误差分析：https://developer.nvidia.com/blog/solving-self-intersection-artifacts-in-directx-raytracing/#connection_rays
public bool connection_ray(SurfacePoint source, TriangleEndpoint target, out Ray ray)
{
    float3 delta = target.position - source.position;
    ray.origin = offset_origin(source, delta);
    ray.t_min = 0.0;
    precise float3 direction = target.position - ray.origin;
    float offset =
        target.base_offset + dot(target.traversal_weight, abs(ray.origin) + abs(direction));
    if (dot(target.normal, -direction) < 0.0)
        offset = -offset;
    precise float3 adjusted = fma(float3(offset), target.normal, direction);
    ray.direction = adjusted;
    ray.t_max = asfloat(0x3f7fffffu);
    return all(float2(dot(direction, delta), dot(adjusted, delta)) > 0.0);
}
}
