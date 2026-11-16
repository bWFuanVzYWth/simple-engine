// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-FileCopyrightText: Copyright (c) 1998-2020 Matt Pharr, Wenzel Jakob, and Greg Humphreys
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
// FastOwen 来自 PBRT-v4；其 Apache-2.0 许可见 ../../LICENSES/pbrt-Apache-2.0.txt。

/// 无资源、表、64 位算术或隐式随机状态的 Z-order / Sobol / FastOwen 采样。
/// 算法来源：https://doi.org/10.1145/3414685.3417881 。整数序列沿用作者 z_sobol，
/// 不等同 PBRT 原始 index hash。完整 net 的均匀性不保证任意早期前缀的质量。
module z_sobol;
import pt.sobol_bits;

namespace pt
{

func permute_four(digit: uint, hash: uint)->uint
{
    let pair : uint = ((hash >> 8) * 6u) >> 24;
    let a : uint = (0x0fa5u >> (2u * pair)) & 3u;
    let b : uint = (0x09deu >> (2u * pair)) & 3u;
    return ((digit & 1u) * a) ^ ((digit >> 1) * b) ^ (hash & 3u);
}

func node_hash(prefix: uint2, shift: uint, key: uint)->uint
{
    return sobol_bits::hash(prefix.x ^ (prefix.y * 0x9e3779b9u) ^ key ^ (shift * 0x632be59bu));
}

func permuted_index(morton: uint2, bits: uint, odd: uint, key: uint)->uint2
{
    var result = uint2(0);
    if (bits <= 32)
    {
        var m : uint = morton.x >> odd;
        if (odd != 0)
            result.x = (morton.x & 1u) ^ (node_hash(uint2(m, 0), 0, key) & 1u);
        for (var shift: uint = odd; shift < bits; shift += 2)
        {
            let digit : uint = m & 3u;
            m >>= 2;
            result.x |= permute_four(digit, node_hash(uint2(m, 0), shift, key)) << shift;
        }
    }
    else
    {
        var m : uint2 = morton;
        if (odd != 0)
        {
            m = uint2((m.x >> 1) | (m.y << 31), m.y >> 1);
            result.x = (morton.x & 1u) ^ (node_hash(m, 0, key) & 1u);
        }
        for (var shift: uint = odd; shift < bits; shift += 2)
        {
            let digit : uint = m.x & 3u;
            m = uint2((m.x >> 2) | (m.y << 30), m.y >> 2);
            let d = permute_four(digit, node_hash(m, shift, key));
            if (shift < 32)
            {
                result.x |= d << shift;
                if (shift == 31)
                    result.y |= d >> 1;
            }
            else
                result.y |= d << (shift - 32);
        }
    }
    return result;
}

/// 一个像素／样本的值类型；domain 是固定语义身份，调用顺序不推进随机流。
public struct ZSobol
{
    internal var morton : uint2;
    internal var bit_count : uint;
    internal var odd : uint;
    internal var seed : uint;

    /// 返回同一 domain 的两个 32 位 Sobol/Owen 分量。
    public func bits_2d(domain: uint)->uint2
    {
        let key = sobol_bits::hash(domain ^ seed);
        let index = permuted_index(morton, bit_count, odd, key);
        return uint2(sobol_bits::owen_reversed(index.x, sobol_bits::hash(key ^ 0xa511e9b3u)),
                     sobol_bits::owen_reversed(sobol_bits::reversed_y(index.x ^ index.y),
                                               sobol_bits::hash(key ^ 0x63d83595u)));
    }

    /// 与 bits_2d(domain).x 相同，不构成额外随机维度。
    public func bits_1d(domain: uint)->uint
    {
        let key = sobol_bits::hash(domain ^ seed);
        let index = permuted_index(morton, bit_count, odd, key);
        return sobol_bits::owen_reversed(index.x, sobol_bits::hash(key ^ 0xa511e9b3u));
    }

    /// 用高 24 位返回 [0,1)²；不把舍入后的 1 当作合法样本。
    public func sample_2d(domain: uint)->float2
    {
        return float2(bits_2d(domain) >> 8) * (1.0 / 16777216.0);
    }

    /// 与 sample_2d(domain).x 相同，范围 [0,1)。
    public func sample_1d(domain: uint)->float
    {
        return float(bits_1d(domain) >> 8) * (1.0 / 16777216.0);
    }
};

/// R<=16、S<=20、2R+S<=52，pixel 各分量<2^R、sample<2^S；不支持其他输入，不校验。
/// 一个完整样本集固定 R/S/seed/domain；不能随累计样本数改变 S。
public func z_sobol(pixel: uint2, sample_value: uint, r: uint, s: uint, seed: uint)->ZSobol
{
    let m : uint = sobol_bits::spread(pixel.x) | (sobol_bits::spread(pixel.y) << 1);
    var result : ZSobol;
    if (s == 0)
        result.morton = uint2(m, 0);
    else
        result.morton = uint2((m << s) | sample_value, m >> (32 - s));
    result.bit_count = 2 * r + s;
    result.odd = s & 1u;
    result.seed = seed;
    return result;
}

/// S=8 的相同序列；sample<256，其余边界同 z_sobol。
/// 超过 256 个绝对样本后由应用更换全局 scramble，开始新完整 net，不扩展原 net。
public func z_sobol_s8(pixel: uint2, sample_value: uint, r: uint, seed: uint)->ZSobol
{
    let m : uint = sobol_bits::spread(pixel.x) | (sobol_bits::spread(pixel.y) << 1);
    var result : ZSobol;
    result.morton = uint2((m << 8) | sample_value, m >> 24);
    result.bit_count = 2 * r + 8;
    result.odd = 0;
    result.seed = seed;
    return result;
}
}
