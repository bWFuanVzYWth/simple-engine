module dag;

static const float DDA_INF = asfloat(0x7f800000u);

/// 64 叉 DAG 的只读设备视图。编号仅在该资源版本和对应层中有效；数组必须覆盖可达节点。
public struct DagView
{
    public StructuredBuffer<uint> values;
    public StructuredBuffer<uint2> masks;
    public uint4 offsets[3];
    public uint2 root; // 指针标记、payload
};

/// 均匀叶；lod 是边长的二进制指数，0 表示单体素。
public struct DagSample
{
    public uint value;
    public uint lod;
    public bool valid;
};

/// 每条射线独占的祖先缓存；初始化为零，仅在同一 DagView 上复用。
public struct DagPath
{
    public uint ancestors[10];
    public uint3 voxel;
    public DagSample sample;
};

uint slot_at(uint3 voxel, uint level)
{
    uint3 p = voxel >> (level * 2);
    return (p.x & 1) | ((p.y & 1) << 1) | ((p.z & 1) << 2) | ((p.x & 2) << 2) | ((p.y & 2) << 3) |
           ((p.z & 2) << 4);
}

/// 读取各轴位于 [0, 1<<20) 的均匀叶；不分配、不赋予叶材料含义。
/// view 的可达指针与层偏移须有效且索引计算不得超过 uint；path 不能跨资源版本复用。
public DagSample uniform_at(DagView view, uint3 voxel, inout DagPath path)
{
    uint2 child = view.root;
    uint level = 9;
    if (path.sample.valid)
    {
        uint3 different = voxel ^ path.voxel;
        uint changed = different.x | different.y | different.z;
        if (changed < (1u << path.sample.lod))
            return path.sample;
        level = uint(firstbithigh(changed)) / 2;
        child = uint2(1, path.ancestors[level]);
    }
    path.voxel = voxel;
    [loop]
    for (;;)
    {
        if (child.x == 0)
        {
            path.sample = { child.y, (level + 1) * 2, true };
            return path.sample;
        }
        path.ancestors[level] = child.y;
        uint index = view.offsets[level / 4][level % 4] + child.y;
        uint slot = slot_at(voxel, level);
        uint bits = view.masks[index][slot / 32];
        child = uint2((bits >> (slot & 31)) & 1, view.values[index * 64 + slot]);
        if (level == 0)
            break;
        --level;
    }
    path.sample = { child.y, 0, child.x == 0 };
    return path.sample;
}

/// 整数基点与局部小数分开，避免大坐标消减抹掉体素内精度。
/// DDA 的 base、盒边界和跳跃区域边界各轴须在 [-2^20, 2^20]，fraction 位于 [0,1)。
/// 此域保证边界减 base、一步 crossing 及 region 加法不溢出 int；不保证 float 等同实数。
public struct RayOrigin
{
    public int3 base;
    public float3 fraction;
};

/// 迭代 DDA 状态；边界时间始终从原始射线重算，等时按 X/Y/Z 顺序推进。
public struct DdaState
{
    public int3 voxel;
    public int3 step;
    public int3 next_boundary;
    public float t;
    public int axis;
};

/// 从有限且 floor 位于 RayOrigin 域内的浮点坐标拆分整数基点。
public RayOrigin split_origin(float3 origin)
{
    RayOrigin result;
    result.base = int3(floor(origin));
    result.fraction = origin - float3(result.base);
    return result;
}

float boundary_t(int origin_base, float fraction, float inverse, int boundary, int step)
{
    if (step == 0)
        return DDA_INF;
    return (float(boundary - origin_base) - fraction) * inverse;
}

int argmin3(float3 value)
{
    if (value.x <= value.y && value.x <= value.z)
        return 0;
    if (value.y <= value.z)
        return 1;
    return 2;
}

/// 与 RayOrigin 域内的整数半开盒求交；各轴 lower<upper，dir 有限且整体非零。
/// 非零方向的倒数须有限，max_t 有限非负；t 是原 dir 的参数，不自动归一化。
/// 最大 t 包含命中，纯相切不入盒；max_t=0 保留盒内起点的立即命中。
public bool begin_dda(RayOrigin origin, float3 dir, int3 lower, int3 upper, float max_t,
                      out DdaState state, out float end)
{
    float3 near_t = -DDA_INF;
    float3 far_t = DDA_INF;
    state = (DdaState)0;
    [unroll]
    for (int axis = 0; axis < 3; ++axis)
    {
        state.step[axis] = dir[axis] > 0 ? 1 : (dir[axis] < 0 ? -1 : 0);
        if (state.step[axis] == 0)
        {
            if (origin.base[axis] < lower[axis] || origin.base[axis] >= upper[axis])
            {
                end = 0;
                return false;
            }
        }
        else
        {
            float inverse = 1.0 / dir[axis];
            float lo = boundary_t(origin.base[axis], origin.fraction[axis], inverse, lower[axis],
                                  state.step[axis]);
            float hi = boundary_t(origin.base[axis], origin.fraction[axis], inverse, upper[axis],
                                  state.step[axis]);
            near_t[axis] = min(lo, hi);
            far_t[axis] = max(lo, hi);
        }
    }
    state.t = max(max(max(near_t.x, near_t.y), near_t.z), 0.0);
    float box_end = min(min(far_t.x, far_t.y), far_t.z);
    end = min(box_end, max_t);
    if (end < state.t || end < 0.0)
        return false;
    if (box_end == state.t &&
        (state.t > 0.0 || any(origin.base < lower) || any(origin.base >= upper)))
        return false;
    state.axis = -1;
    if (state.t > 0)
    {
        if (near_t.x >= near_t.y && near_t.x >= near_t.z)
            state.axis = 0;
        else
            state.axis = near_t.y >= near_t.z ? 1 : 2;
    }
    state.voxel = origin.base + int3(floor(origin.fraction + dir * state.t));
    [unroll]
    for (int axis = 0; axis < 3; ++axis)
    {
        if (near_t[axis] == state.t)
            state.voxel[axis] = state.step[axis] > 0 ? lower[axis] : upper[axis] - 1;
        state.next_boundary[axis] = state.voxel[axis] + (state.step[axis] > 0 ? 1 : 0);
    }
    return all(state.voxel >= lower) && all(state.voxel < upper);
}

float3 boundary_times(DdaState state, float3 inverse, int3 boundary, RayOrigin origin)
{
    float3 result;
    [unroll]
    for (int axis = 0; axis < 3; ++axis)
        result[axis] = boundary_t(origin.base[axis], origin.fraction[axis], inverse[axis],
                                  boundary[axis], state.step[axis]);
    return result;
}

/// 下一单体素边界的参数；不修改状态。
/// state 来自本模块，origin/dir 与初始化相同；inverse 非零轴为 1/dir，零方向轴置 0。
/// 后续 advance/skip 沿用此约定，调用者在越出盒或达到最大 t 后停止。
public float next_t(DdaState state, float3 inverse, RayOrigin origin)
{
    float3 times = boundary_times(state, inverse, state.next_boundary, origin);
    return times[argmin3(times)];
}

/// 前进一个单体素 crossing；即使多个轴等时也只推进优先轴。
public DdaState advance_one(DdaState state, float3 inverse, RayOrigin origin)
{
    float3 times = boundary_times(state, inverse, state.next_boundary, origin);
    int axis = argmin3(times);
    state.voxel[axis] += state.step[axis];
    state.next_boundary[axis] += state.step[axis];
    state.t = times[axis];
    state.axis = axis;
    return state;
}

bool crossing_reaches(int axis, float inverse, int boundary, int step, uint crossing, float limit,
                      bool equal, RayOrigin origin)
{
    float t = boundary_t(origin.base[axis], origin.fraction[axis], inverse,
                         boundary + step * int(crossing), step);
    return equal ? t <= limit : t < limit;
}

uint crossings_before(int axis, float inverse, int boundary, int step, uint count, float limit,
                      bool equal, RayOrigin origin)
{
    if (step == 0 || count == 0)
        return 0;
    if (!crossing_reaches(axis, inverse, boundary, step, 0, limit, equal, origin))
        return 0;
    if (count == 1 ||
        crossing_reaches(axis, inverse, boundary, step, count - 1, limit, equal, origin))
        return count;
    float first = boundary_t(origin.base[axis], origin.fraction[axis], inverse, boundary, step);
    float estimated = floor((limit - first) / abs(inverse)) + 1.0;
    if (estimated >= 0 && estimated <= float(count) + 2.0)
    {
        uint candidate = uint(clamp(estimated, 1.0, float(count - 1)));
        [unroll]
        for (uint correction = 0; correction < 2; ++correction)
        {
            if (!crossing_reaches(axis, inverse, boundary, step, candidate - 1, limit, equal,
                                  origin))
                --candidate;
            else if (!crossing_reaches(axis, inverse, boundary, step, candidate, limit, equal,
                                       origin))
                return candidate;
            else
                ++candidate;
        }
    }
    uint lo = 0, hi = count;
    [loop]
    while (lo < hi)
    {
        uint mid = (lo + hi + 1) / 2;
        if (crossing_reaches(axis, inverse, boundary, step, mid - 1, limit, equal, origin))
            lo = mid;
        else
            hi = mid - 1;
    }
    return lo;
}

/// 跳过对齐均匀区域；region_min 与 edge 由调用者按坐标映射计算。
/// 出口处仍保持逐格 advance_one 的 XYZ tie 顺序，不能浮点重建全部 voxel。
/// edge 在 1..=2^20，区域边界在 RayOrigin 域内且包含 state.voxel；只跳过调用者允许的叶。
public DdaState skip_region(DdaState state, float3 inverse, RayOrigin origin, int3 region_min,
                            uint edge)
{
    int3 region_max = region_min + int(edge);
    int3 boundary;
    uint3 max_count;
    [unroll]
    for (int axis = 0; axis < 3; ++axis)
    {
        boundary[axis] = state.step[axis] > 0 ? region_max[axis] : region_min[axis];
        max_count[axis] =
            state.step[axis] > 0
                ? uint(region_max[axis] - state.voxel[axis])
                : (state.step[axis] < 0 ? uint(state.voxel[axis] - region_min[axis] + 1) : 0);
    }
    float3 times = boundary_times(state, inverse, boundary, origin);
    int face = argmin3(times);
    float t = times[face];
    [unroll]
    for (int axis = 0; axis < 3; ++axis)
    {
        uint count = axis == face
                         ? max_count[axis]
                         : (max_count[axis] <= 1
                                ? 0
                                : crossings_before(axis, inverse[axis], state.next_boundary[axis],
                                                   state.step[axis], max_count[axis] - 1, t,
                                                   axis < face, origin));
        int delta = state.step[axis] * int(count);
        state.voxel[axis] += delta;
        state.next_boundary[axis] += delta;
    }
    state.t = t;
    state.axis = face;
    return state;
}
