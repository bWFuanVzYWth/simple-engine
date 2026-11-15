// 来源：Sky Tracer 9e9430d；测试生产共享函数的球形边界与常系数传输恒等式。
import atmosphere.parameters;
import atmosphere.sampling;
using namespace atmosphere;
#include "atmosphere/shadow_profile.inc.slang"

[[vk::binding(0, 0)]]
var inputs : StructuredBuffer<uint4>;
[[vk::binding(1, 0)]]
var outputs : RWStructuredBuffer<uint4>;

[shader("compute")]
[numthreads(64, 1, 1)]
func atmosphere_main(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a : float4 = asfloat(inputs[2 * id.x]);
    let b : uint4 = inputs[2 * id.x + 1];
    if (b.y == 0)
        outputs[id.x] = asuint(segment_integral(a, asfloat(b.x)));
    else if (b.y == 2)
        outputs[id.x] = asuint(segment_transport(a, asfloat(b.x)).transmittance);
    else
    {
        var p : Params = (Params)0;
        p.sun.w = 6360.0;
        p.medium.x = 120.0;
        let hit : bool = ground(p, a.x, a.y);
        let d : float = boundary(p, a.x, a.y, hit);
        outputs[id.x] = uint4(asuint(float3(d, height_at(p, a.x, a.y, d), float(hit))), id.x + 1);
    }
}

struct IntervalProfileSource : IPrimeSunShadowProfileSource
{
    func texelSize(cascade: int)->float
    {
        return sunShadowTexelSize(cascade);
    }
    func blockerDepth(cascade: int, node: int2)->float
    {
        return 0;
    }
};
[shader("compute")]
[numthreads(1, 1, 1)]
func shadow_interval_main(uint3 id: SV_DispatchThreadID)->void
{
    var rows : uint;
    var stride : uint;
    inputs.GetDimensions(rows, stride);
    if (id.x >= rows / 2)
        return;
    let a = asfloat(inputs[2 * id.x]);
    let b = asfloat(inputs[2 * id.x + 1]);
    primeInitializeSunShadowProfile(float3(0, 0, 1), float2(0), a.w, float3(1, 0, 0),
                                    float3(1, 0, 0), float3(0, 1, 0));
    sunShadowProfileCounts[0] = 3;
    sunShadowProfileEntries[0] = float2(63.5, -0.25);
    sunShadowProfileEntries[1] = float2(95.25, 0.5);
    sunShadowProfileEntries[2] = float2(127.0, 1.25);
    for (var cascade = 1; cascade < SUN_SHADOW_CASCADE_COUNT; ++cascade)
    {
        sunShadowProfileCounts[cascade] = 1;
        sunShadowProfileEntries[sunShadowProfileBase(cascade)] =
            float2(sunShadowProfileCascadeEnd(IntervalProfileSource(), cascade), 0.35);
    }
    let visibility =
        localSunVisibilityInterval(IntervalProfileSource(), sunShadowProfileRay(a.xyz), b.x, b.y);
    outputs[id.x] = uint4(asuint(visibility), 0, 0, id.x + 1);
}
