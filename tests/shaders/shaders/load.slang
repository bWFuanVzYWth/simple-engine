module load;

import geometry.ray;

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<float4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<float4>;

// 一次连接构造的批量消费者；输入决定尺度、切向分布与工作集，不循环反馈。
[numthreads(64, 1, 1)]
func connection_load_main(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 5)
        return;
    let start : uint = id.x * 5;
    let target : geometry::TriangleEndpoint =
                     geometry::triangle_endpoint(inputs[start].xyz, inputs[start + 1].xyz,
                                                 inputs[start + 2].xyz, inputs[start + 3].xy);
    var source : geometry::SurfacePoint;
    source.position = inputs[start + 4].xyz;
    source.normal = -target.normal;
    source.offset = inputs[start + 4].w;
    var ray : geometry::Ray;
    let valid : bool = geometry::connection_ray(source, target, ray);
    outputs[id.x] = float4(ray.direction, valid ? ray.t_max : -1);
}
