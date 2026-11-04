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
public static const uint PAIRED_NEIGHBOR_COUNT = 3;
public static const uint PAIRED_SLOT_CAPACITY = 65536;

/// 原NV每slot的周期；此处不能拿256替代PNG的实际边长。
public uint paired_size(uint slot)
{
    uint3 sizes = uint3(254, 232, 196);
    return sizes[slot];
}

/// NV随机平移、反射和转置；randomBits来自整帧共享RNG的一次next()。
public struct PairedNeighborTransform
{
    public int offsetX;
    public int offsetY;
    public bool flipX;
    public bool flipY;
    public bool transpose;
};

/// 保留NV 15+14+1+1+1bit分割；frameDim须非零，不按像素独立播种。
public PairedNeighborTransform paired_transform(uint randomBits, uint2 frameDim)
{
    PairedNeighborTransform result;
    result.offsetX = int((randomBits & 0x7fffu) % frameDim.x);
    randomBits >>= 15;
    result.offsetY = int((randomBits & 0x3fffu) % frameDim.y);
    randomBits >>= 14;
    result.flipX = (randomBits & 1u) != 0;
    randomBits >>= 1;
    result.flipY = (randomBits & 1u) != 0;
    randomBits >>= 1;
    result.transpose = (randomBits & 1u) != 0;
    return result;
}

/// 读取同slot原LUT后逆变换delta；pixel为有效非负屏幕像素，slot<3。
/// lut须完整保留原像素，不可替换为独立随机offset；两次同slot应用得到原像素。
public int2 paired_delta(StructuredBuffer<uint> lut, PairedNeighborTransform transform, uint slot,
                         uint2 pixel)
{
    int size = int(paired_size(slot));
    int2 coordinate = (int2(pixel) + int2(transform.offsetX, transform.offsetY)) % size;
    if (transform.flipX)
        coordinate.x = size - 1 - coordinate.x;
    if (transform.flipY)
        coordinate.y = size - 1 - coordinate.y;
    if (transform.transpose)
        coordinate = coordinate.yx;
    uint packed = lut[PAIRED_SLOT_CAPACITY * slot + uint(coordinate.y * size + coordinate.x)];
    int2 delta = int2(int(packed & 0xffu) - 128, int(packed >> 8) - 128);
    if (transform.transpose)
        delta = delta.yx;
    if (transform.flipX)
        delta.x = -delta.x;
    if (transform.flipY)
        delta.y = -delta.y;
    return delta;
}

/// 有界屏幕不做periodic wrap；越界邻居由workload pass拒绝，避免跨画面边缘重用。
public int2 paired_neighbor(StructuredBuffer<uint> lut, PairedNeighborTransform transform,
                            uint slot, uint2 pixel)
{
    return int2(pixel) + paired_delta(lut, transform, slot, pixel);
}

/// paired precomputed/reconnection逻辑地址。逆向缓存取neighborOffset的同一个slot。
public uint paired_address(uint reservoirOffset, uint slot)
{
    return PAIRED_NEIGHBOR_COUNT * reservoirOffset + slot;
}
}
