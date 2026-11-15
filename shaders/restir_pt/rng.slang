/***************************************************************************
 # Copyright (c) 2015-23, NVIDIA CORPORATION. All rights reserved.
 #
 # Redistribution and use in source and binary forms, with or without
 # modification, are permitted provided that the following conditions
 # are met:
 #  * Redistributions of source code must retain the above copyright
 #    notice, this list of conditions and the following disclaimer.
 #  * Redistributions in binary form must reproduce the above copyright
 #    notice, this list of conditions and the following disclaimer in the
 #    documentation and/or other materials provided with the distribution.
 #  * Neither the name of NVIDIA CORPORATION nor the names of its
 #    contributors may be used to endorse or promote products derived
 #    from this software without specific prior written permission.
 #
 # THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS "AS IS" AND ANY
 # EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
 # IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR
 # PURPOSE ARE DISCLAIMED.  IN NO EVENT SHALL THE COPYRIGHT OWNER OR
 # CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL,
 # EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO,
 # PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR
 # PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY
 # OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
 # (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
 # OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
 **************************************************************************/

/// Falcor TinyUniformSampleGenerator：16-round TEA像素播种+32-bit Numerical Recipes LCG。
/// 保留原随机序列及upper24浮点映射；质量限制沿用NV说明，不能宣称等价高维低差异采样。
module rng;

namespace restir_pt
{
/// 把像素低16bits交错成32-bit Morton code，与NV interleave_32bit完全一致。
public func interleave32(pixel: uint2)->uint
{
    var value : uint2 = pixel & 0xffffu;
    value = (value | (value << 8)) & 0x00ff00ffu;
    value = (value | (value << 4)) & 0x0f0f0f0fu;
    value = (value | (value << 2)) & 0x33333333u;
    value = (value | (value << 1)) & 0x55555555u;
    return value.x | (value.y << 1);
}

/// NV固定128-bit key的16-round TEA；所有uint运算按2^32环绕。
public func block_cipher_tea(first: uint, second: uint)->uint2
{
    var first = first;
    var second = second;
    var sum : uint = 0;
    for (var i: uint = 0; i < 16; ++i)
    {
        sum += 0x9e3779b9u;
        first += ((second << 4) + 0xa341316cu) ^ (second + sum) ^ ((second >> 5) + 0xc8013ea4u);
        second += ((first << 4) + 0xad90777du) ^ (first + sum) ^ ((first >> 5) + 0x7e95761eu);
    }
    return uint2(first, second);
}

/// 完整状态仅4B；初采保存消费前state，replay由相同state恢复并按相同分支消费。
public struct TinyUniformSampleGenerator
{
    public var state : uint;
    [mutating]
    public func next()->uint
    {
        state = 1664525u * state + 1013904223u;
        return state;
    }
    [mutating]
    public func next1D()->float
    {
        return float(next() >> 8) * 0x1p-24f;
    }
    [mutating]
    public func next2D()->float2
    {
        var value : float2;
        value.x = next1D();
        value.y = next1D();
        return value;
    }
    [mutating]
    public func next3D()->float3
    {
        var value : float3;
        value.x = next1D();
        value.y = next1D();
        value.z = next1D();
        return value;
    }
    [mutating]
    public func next4D()->float4
    {
        var value : float4;
        value.x = next1D();
        value.y = next1D();
        value.z = next1D();
        value.w = next1D();
        return value;
    }
};

/// 由原始LCG状态恢复，不额外hash；用于prefix/rc后缀的精确随机重放。
public func rng_from_seed(seed: uint)->TinyUniformSampleGenerator
{
    var result : TinyUniformSampleGenerator;
    result.state = seed;
    return result;
}

/// NV像素/样本播种：TEA(interleave32(pixel),sampleNumber).x，随后才开始LCG。
public func rng_for_pixel(pixel: uint2, sampleNumber: uint)->TinyUniformSampleGenerator
{
    return rng_from_seed(block_cipher_tea(interleave32(pixel), sampleNumber).x);
}
}
