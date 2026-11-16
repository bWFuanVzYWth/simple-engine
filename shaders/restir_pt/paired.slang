/***************************************************************************
 # Copyright (c) 2015-26, NVIDIA CORPORATION. All rights reserved.
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

/// Falcor 759aad03原PairedNeighborTransform及默认K=3、stdev16.0 LUT。
/// paired.bin每slot65536个little-endian u32；真实边长254/232/196，其余元素为0。
/// 同帧同round全部像素必须共用同一transform；每个slot是自身的逆映射。
module paired;

namespace restir_pt
{
public static const let PAIRED_NEIGHBOR_COUNT : uint = 3;
public static const let PAIRED_SLOT_CAPACITY : uint = 65536;

/// 原NV每slot的周期；此处不能拿256替代PNG的实际边长。
public func paired_size(slot: uint)->uint
{
    let sizes = uint3(254, 232, 196);
    return sizes[slot];
}

/// NV随机平移、反射和转置；randomBits来自整帧共享RNG的一次next()。
public struct PairedNeighborTransform
{
    public var offset_x : int;
    public var offset_y : int;
    public var flip_x : bool;
    public var flip_y : bool;
    public var transpose : bool;
};

/// 保留NV 15+14+1+1+1bit分割；frameDim须非零，不按像素独立播种。
public func paired_transform(random_bits: uint, frame_dim: uint2)->PairedNeighborTransform
{
    let random_bits = random_bits;
    var result : PairedNeighborTransform;
    result.offset_x = int((random_bits & 0x7fffu) % frame_dim.x);
    random_bits >>= 15;
    result.offset_y = int((random_bits & 0x3fffu) % frame_dim.y);
    random_bits >>= 14;
    result.flip_x = (random_bits & 1u) != 0;
    random_bits >>= 1;
    result.flip_y = (random_bits & 1u) != 0;
    random_bits >>= 1;
    result.transpose = (random_bits & 1u) != 0;
    return result;
}

/// 读取同slot原LUT后逆变换delta；pixel为有效非负屏幕像素，slot<3。
/// lut须完整保留原像素，不可替换为独立随机offset；两次同slot应用得到原像素。
public func paired_delta(lut: StructuredBuffer<uint>, transform: PairedNeighborTransform,
                         slot: uint, pixel: uint2)
    ->int2
{
    let size = int(paired_size(slot));
    var coordinate : int2 = (int2(pixel) + int2(transform.offset_x, transform.offset_y)) % size;
    if (transform.flip_x)
        coordinate.x = size - 1 - coordinate.x;
    if (transform.flip_y)
        coordinate.y = size - 1 - coordinate.y;
    if (transform.transpose)
        coordinate = coordinate.yx;
    let packed = lut[PAIRED_SLOT_CAPACITY * slot + uint(coordinate.y * size + coordinate.x)];
    var delta = int2(int(packed & 0xffu) - 128, int(packed >> 8) - 128);
    if (transform.transpose)
        delta = delta.yx;
    if (transform.flip_x)
        delta.x = -delta.x;
    if (transform.flip_y)
        delta.y = -delta.y;
    return delta;
}

/// 有界屏幕不做periodic wrap；越界邻居由workload pass拒绝，避免跨画面边缘重用。
public func paired_neighbor(lut: StructuredBuffer<uint>, transform: PairedNeighborTransform,
                            slot: uint, pixel: uint2)
    ->int2
{
    return int2(pixel) + paired_delta(lut, transform, slot, pixel);
}

/// paired precomputed/reconnection逻辑地址。逆向缓存取neighborOffset的同一个slot。
public func paired_address(reservoir_offset: uint, slot: uint)->uint
{
    return PAIRED_NEIGHBOR_COUNT * reservoir_offset + slot;
}
}
