module load;

import geometry.ray;

[[vk::binding(0, 0)]]
StructuredBuffer<float4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> outputs;

// 一次连接构造的批量消费者；输入决定尺度、切向分布与工作集，不循环反馈。
[numthreads(64, 1, 1)]
void connection_load_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 5)
        return;
    uint start = id.x * 5;
    geometry::TriangleEndpoint target = geometry::triangle_endpoint(
        inputs[start].xyz, inputs[start + 1].xyz, inputs[start + 2].xyz, inputs[start + 3].xy);
    geometry::SurfacePoint source;
    source.position = inputs[start + 4].xyz;
    source.normal = -target.normal;
    source.offset = inputs[start + 4].w;
    geometry::Ray ray;
    bool valid = geometry::connection_ray(source, target, ray);
    outputs[id.x] = float4(ray.direction, valid ? ray.t_max : -1);
}
