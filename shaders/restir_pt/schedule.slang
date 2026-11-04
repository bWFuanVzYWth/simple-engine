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

/// NV 16x16 tile/Morton索引与Compact retrace队列；不决定scene几何有效性。
/// compact_append要求[numthreads(256,1,1)]及[WaveSize(32)]，所有线程都到达两个group barrier。
module schedule;
import restir_pt.rng;

namespace restir_pt
{
public static const uint SCREEN_TILE_DIM = 16;
public static const uint SCREEN_TILE_THREADS = 256;
public static const uint DEFAULT_NEIGHBOR_COUNT = 3;
public static const uint DEFAULT_HISTORY_LENGTH = 20;
public static const uint DEFAULT_SPATIAL_ROUNDS = 1;

/// host与shader须使用同一tile数量；frameDim各分量在1..4096，容量乘法须能用uint表示。
public uint2 screen_tiles(uint2 frameDim)
{
    return (frameDim + 15u) >> 4;
}

/// tile row-major、tile内Morton；全屏数组含完整边缘tile，不使用W*H直接当reservoir容量。
public uint reservoir_offset(uint2 pixel, uint2 tiles)
{
    uint2 tile = pixel >> 4;
    return (tile.y * tiles.x + tile.x) * 256u + (interleave32(pixel) & 255u);
}

/// 256-thread group内lane按Morton映射16x16像素；不含screen边缘检查。
public uint2 pixel_from_tile(uint2 tile, uint threadIndex)
{
    uint value = ((threadIndex >> 1) << 8) | threadIndex;
    value &= 0x00005555u;
    value = (value ^ (value >> 1)) & 0x33333333u;
    value = (value ^ (value >> 2)) & 0x0f0f0f0fu;
    return uint2(value & 15u, value >> 8) + (tile << 4);
}

/// 同时拒绝负坐标及超出屏幕的paired/reprojection目标。
public bool valid_pixel(int2 pixel, uint2 frameDim)
{
    return all(pixel >= 0) && all(pixel < int2(frameDim));
}

/// NV默认point nearest重投影；motion为归一化屏幕位移，输入/prev相机约定归宿主。
/// cast使用NV向0截断语义；调用后还须valid_pixel，不在此钳到边缘。
public int2 reproject_point(uint2 pixel, float2 motion, uint2 frameDim)
{
    return int2(float2(pixel) + motion * float2(frameDim) + 0.5f);
}

/// 每个work item只有4B；pixel.xy各12bits，retraceIndex为高8bits，调用方保证范围。
public uint encode_work_item(uint2 pixel, uint retraceIndex)
{
    return pixel.x | (pixel.y << 12) | (retraceIndex << 24);
}

/// 返回(x,y,retraceIndex)；此uint3只是线程内解码值，不是队列元素stride。
public uint3 decode_work_item(uint item)
{
    return uint3(item & 0xfffu, (item >> 12) & 0xfffu, item >> 24);
}

/// temporal双向逻辑地址：0 canonical->history，1 history->canonical。
public uint temporal_address(uint reservoirOffset, uint direction)
{
    return 2u * reservoirOffset + direction;
}

/// paired空间slot只发forward工作；逆方向复用另一像素的同slot缓存。
public uint paired_work_bit(uint slot)
{
    return 1u << (2u * slot + 1u);
}

/// 只在本次256-thread workload group的两个barrier之间使用，不跨dispatch保留。
groupshared uint gCompactWaveOffset[8];

/// 两级Wave/group prefix，整个group仅一次原子分配；保留NV warp顺序及slot顺序。
/// mask仅含低slotCount bits（point temporal为2，paired K3空间为6）；无效像素传mask=0。
/// counters至少4B且dispatch前清零，items容量须覆盖本轮最坏workitem数；不在此溢出丢工作。
public void compact_append(uint threadIndex, uint2 pixel, uint mask, uint slotCount,
                           RWByteAddressBuffer counters, RWByteAddressBuffer items)
{
    uint count = countbits(mask);
    uint warpIndex = threadIndex >> 5;
    uint waveCount = WaveActiveSum(count);
    if (WaveIsFirstLane())
        gCompactWaveOffset[warpIndex] = waveCount;
    GroupMemoryBarrierWithGroupSync();
    if (threadIndex < 8)
    {
        uint value = gCompactWaveOffset[threadIndex];
        uint sum = WaveActiveSum(value);
        uint prefix = WavePrefixSum(value);
        uint original = 0;
        if (WaveIsFirstLane())
            counters.InterlockedAdd(0, sum, original);
        original = WaveReadLaneFirst(original);
        gCompactWaveOffset[threadIndex] = original + prefix;
    }
    GroupMemoryBarrierWithGroupSync();
    uint destination = gCompactWaveOffset[warpIndex] + WavePrefixSum(count);
    uint written = 0;
    for (uint index = 0; index < slotCount; ++index)
    {
        if (((mask >> index) & 1u) != 0)
        {
            items.Store(4u * (destination + written), encode_work_item(pixel, index));
            ++written;
        }
    }
}

/// 消费端按最坏容量dispatch，计数之外线程退出；读取前需要UAV执行/内存依赖。
public bool compact_item(ByteAddressBuffer counters, ByteAddressBuffer items, uint linearIndex,
                         out uint3 decoded)
{
    decoded = 0;
    if (linearIndex >= counters.Load(0))
        return false;
    decoded = decode_work_item(items.Load(4u * linearIndex));
    return true;
}
}
