// 散乱 hint 重排须保留 dispatch 身份和跨重排的局部值。
[[vk::binding(0, 0)]]
RWStructuredBuffer<uint> result;
[[vk::binding(1, 0)]]
StructuredBuffer<uint> source;

[shader("raygeneration")]
void reorder_raygen()
{
    uint index = DispatchRaysIndex().x;
    uint value = source[index] ^ 0x13579bdf;
    ReorderThread(value & 7u, 3u);
    result[index * 2] = value;
    result[index * 2 + 1] = DispatchRaysIndex().x;
}

[shader("miss")]
void reorder_miss(inout uint payload)
{
}

[shader("closesthit")]
void reorder_closest(inout uint payload, BuiltInTriangleIntersectionAttributes attributes)
{
}
