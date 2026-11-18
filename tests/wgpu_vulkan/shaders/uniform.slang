// 不同 descriptor 版本共享程序，uniform 与 storage 的资源类型保持明确。
[[vk::binding(0, 0)]]
var parameters : ConstantBuffer<uint4>;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<uint>;

[shader("compute")]
[numthreads(1, 1, 1)]
func uniform_main()->void {
    result[0] = parameters.x;
}
