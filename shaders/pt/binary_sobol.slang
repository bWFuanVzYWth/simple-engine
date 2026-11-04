// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-FileCopyrightText: Copyright (c) 1998-2020 Matt Pharr, Wenzel Jakob, and Greg Humphreys
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// whole-index binary nested 构造；FastOwen 来自 PBRT-v4。
// 第三方许可见 ../../LICENSES/pbrt-Apache-2.0.txt。

/// 固定 S8 的 binary nested Morton/Sobol 原语，供明确分配联合投影的专用积分器使用。
/// 不声称通用高维采样器、独立 domain、完整 3D net 或时间蓝噪声。
/// FastOwen 来源：https://www.jcgt.org/published/0009/04/01/paper.pdf 。
///
/// 每像素保留完整 256 点二维 net 及全部 aligned 2^m 前缀块。binary 树在每个二位节点
/// 只有 8/24 种四元排列，不能继承原 Z Sobol 四叉树的空间随机族／各向同性结论；
/// 现有 toy 频谱未证实一致的各向同性恶化，也没有恢复旋转对称性的理论保证。
///
/// 普通坐标在 aligned 两点上同时翻转最高位，故跨 domain 半区 XOR 固定。
/// 换 seed/domain 或输出 Owen 无法消除此结构；bits_3d 定向牺牲 scalar 两点分层来保护
/// 组内四点的二维半区投影。跨组、跨 bounce、高阶和细尺度相关仍在，RR 不能默认承接代价。
/// 旧 whole 实验的跨域 XOR、单域斜向 cos、3D 乘积与高 spp toy MSE 均有退化，
/// 此修正不修复所有这些缺陷；尤其 pair 的单域点集完全未改。除两点 XOR 外，
/// 上述实测退化的具体成因未完全隔离，不能把随机树差异当作全部退化的已证根因。
/// 对齐块保证不能推及任意滑窗、运动重投影、拒绝历史或去噪后的误差。
module binary_sobol;

namespace pt
{
uint binary_hash(uint value)
{
    value ^= value >> 16;
    value *= 0x7feb352du;
    value ^= value >> 15;
    value *= 0x846ca68bu;
    return value ^ (value >> 16);
}

uint binary_spread(uint value)
{
    value &= 0xffffu;
    value = (value | (value << 8)) & 0x00ff00ffu;
    value = (value | (value << 4)) & 0x0f0f0f0fu;
    value = (value | (value << 2)) & 0x33333333u;
    return (value | (value << 1)) & 0x55555555u;
}

uint binary_owen(uint value, uint seed)
{
    value ^= value * 0x3d20adeau;
    value += seed;
    value *= (seed >> 16) | 1u;
    value ^= value * 0x05526c56u;
    value ^= value * 0x53a22864u;
    return reversebits(value);
}

uint binary_permute(uint value, uint key)
{
    return binary_owen(reversebits(value), key);
}

uint binary_sobol_y(uint value)
{
    value ^= (value >> 1) & 0x55555555u;
    value ^= (value >> 2) & 0x33333333u;
    value ^= (value >> 4) & 0x0f0f0f0fu;
    value ^= (value >> 8) & 0x00ff00ffu;
    value ^= (value >> 16) & 0x0000ffffu;
    return value;
}

/// 无资源、64 位运算或隐式随机状态的 S8 值类型；domain 为固定语义身份。
/// 不同 domain 只分离 scramble，不代表数学独立；同 domain 的 1D 就是 2D.x。
public struct BinarySobolS8
{
    internal uint2 morton;
    internal uint bit_count;
    internal uint seed;

    internal uint2 index(uint key)
    {
        if (bit_count <= 32)
        {
            uint mask = ~0u >> (32 - bit_count);
            return uint2(binary_permute(morton.x, key) & mask, 0);
        }
        uint high_key = binary_hash(morton.y ^ binary_hash(key ^ 0x632be59bu));
        uint high_mask = ~0u >> (64 - bit_count);
        return uint2(binary_permute(morton.x, high_key), binary_permute(morton.y, key) & high_mask);
    }

    /// 同域 Sobol/Owen 两坐标；保持原 whole 实验的二维序列及其已知缺陷。
    public uint2 bits_2d(uint domain)
    {
        uint key = binary_hash(domain ^ seed);
        uint2 i = index(key);
        return uint2(binary_owen(i.x, binary_hash(key ^ 0xa511e9b3u)),
                     binary_owen(binary_sobol_y(i.x ^ i.y), binary_hash(key ^ 0x63d83595u)));
    }

    /// 同域 bits_2d.x；保留两点分层，也保留普通 domain 间的同步翻转缺陷。
    public uint bits_1d(uint domain)
    {
        uint key = binary_hash(domain ^ seed);
        return binary_owen(index(key).x, binary_hash(key ^ 0xa511e9b3u));
    }

    /// xy 与 bits_2d(pair_domain) 相同；z 为 scalar_domain 索引交换低两 bit 后的 X。
    /// 固定 pixel/seed 的每个 aligned 四点块，xyz 任意两维的半区投影覆盖四格各一次：
    /// binary 低两位行是 (1,t)、(1,t^1)，交换后的 scalar 为 (0,1)，恰为三个非零行。
    /// 四点也满足 (0,2,3)-net；三维半区 XOR 在该块固定，与四点 net 并不矛盾。
    /// 更长块没有共同的三维生成结构；例如 R=0、seed=0、domains=(2,1) 的前八点，
    /// (xbits,ybits,zbits)=(0,2,1) 划分仅占 4/8 格，故不是 (0,3,3)-net。
    /// 独立输出 Owen 只能重排 dyadic cells，不能填补空格；细投影／高阶质量仍可能退化。
    /// z 在每个 aligned 两点中同半区，阈值选择可能比普通 1D 更差；仅其 m>=2 的
    /// aligned 2^m 块完整值集合不变。调用者应将这项代价放在可接受的 scalar 消费上，
    /// 先按材质／消费语义选映射，不能根据本次随机事件再选择映射。
    public uint3 bits_3d(uint pair_domain, uint scalar_domain)
    {
        uint key = binary_hash(scalar_domain ^ seed);
        uint i = index(key).x;
        i = (i & ~3u) | ((i & 1u) << 1) | ((i >> 1) & 1u);
        return uint3(bits_2d(pair_domain), binary_owen(i, binary_hash(key ^ 0xa511e9b3u)));
    }

    /// bits_2d 的高 24 位量化为 [0,1)²；不提供连续实数或全部 32 位浮点精度。
    public float2 sample_2d(uint domain)
    {
        return float2(bits_2d(domain) >> 8) * (1.0 / 16777216.0);
    }

    /// bits_1d 的高 24 位量化为 [0,1)，与 sample_2d(domain).x 相同。
    public float sample_1d(uint domain)
    {
        return float(bits_1d(domain) >> 8) * (1.0 / 16777216.0);
    }

    /// bits_3d 的高 24 位量化为 [0,1)³；保留该接口的定向代价与有限投影保证。
    public float3 sample_3d(uint pair_domain, uint scalar_domain)
    {
        return float3(bits_3d(pair_domain, scalar_domain) >> 8) * (1.0 / 16777216.0);
    }
};

/// R<=16，pixel 各轴<2^R、sample<256；其他输入不支持、不校验。
/// 一个完整 net 固定 R/seed/domain；应用每 256 样本换 seed 后是新 net，不是无限延长。
/// 只提供 S8，避免低两 bit 交换在 S<2 时跨越 sample/pixel 边界。
public BinarySobolS8 binary_sobol_s8(uint2 pixel, uint sample, uint r, uint seed)
{
    uint m = binary_spread(pixel.x) | (binary_spread(pixel.y) << 1);
    BinarySobolS8 result;
    result.morton = uint2((m << 8) | sample, m >> 24);
    result.bit_count = 2 * r + 8;
    result.seed = seed;
    return result;
}
}
