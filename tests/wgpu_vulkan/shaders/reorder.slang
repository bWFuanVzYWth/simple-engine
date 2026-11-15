// 散乱 hint 重排须保留 dispatch 身份和跨重排的局部值。
[[vk::binding(0, 0)]]
var result : RWStructuredBuffer<uint>;
[[vk::binding(1, 0)]]
var source : StructuredBuffer<uint>;

[shader("raygeneration")]
func reorder_raygen()->void
{
    let index : uint = DispatchRaysIndex().x;
    let value : uint = source[index] ^ 0x13579bdf;
    ReorderThread(value & 7u, 3u);
    result[index * 2] = value;
    result[index * 2 + 1] = DispatchRaysIndex().x;
}

[shader("miss")]
func reorder_miss(inout payload_value: uint)->void
{
}

[shader("closesthit")]
func reorder_closest(inout payload_value: uint, attributes: BuiltInTriangleIntersectionAttributes)
    ->void
{
}
