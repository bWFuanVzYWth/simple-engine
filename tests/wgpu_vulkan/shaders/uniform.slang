// 不同 descriptor 版本共享程序，uniform 与 storage 的资源类型保持明确。
[[vk::binding(0, 0)]]
ConstantBuffer<uint4> parameters;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint> result;

[shader("compute")]
[numthreads(1, 1, 1)]
void uniform_main()
{
    result[0] = parameters.x;
}
