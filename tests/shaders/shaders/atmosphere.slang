// 来源：Sky Tracer 9e9430d；测试生产共享函数的球形边界与常系数传输恒等式。
import atmosphere.parameters;
import atmosphere.sampling;
using namespace atmosphere;

[[vk::binding(0, 0)]]
StructuredBuffer<uint4> inputs;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint4> outputs;

[shader("compute")]
[numthreads(64, 1, 1)]
void atmosphere_main(uint3 id: SV_DispatchThreadID)
{
    uint rows, stride;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    float4 a = asfloat(inputs[2 * id.x]);
    uint4 b = inputs[2 * id.x + 1];
    if (b.y == 0)
        outputs[id.x] = asuint(segment_integral(a, asfloat(b.x)));
    else
    {
        Params p = (Params)0;
        p.sun.w = 6360.0;
        p.medium.x = 120.0;
        bool hit = ground(p, a.x, a.y);
        float d = boundary(p, a.x, a.y, hit);
        outputs[id.x] = uint4(asuint(float3(d, height_at(p, a.x, a.y, d), float(hit))), id.x + 1);
    }
}
