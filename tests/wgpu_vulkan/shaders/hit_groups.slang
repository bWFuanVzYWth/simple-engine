[[vk::binding(0, 0)]]
RaytracingAccelerationStructure scene;
[[vk::binding(1, 0)]]
RWStructuredBuffer<uint> result;
struct Payload
{
    uint value;
    float distance;
    uint instance;
    uint primitive;
};
interface ITracer<State>
{
    void trace(inout State state, RayDesc ray);
    void shade(inout State state, uint index, inout Payload payload);
};
struct Tracer : ITracer<HitObject>
{
    void trace(inout HitObject state, RayDesc ray)
    {
        uint unused = 0;
        state = HitObject::TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, unused);
    }
    void shade(inout HitObject state, uint index, inout Payload payload)
    {
        payload.distance = state.GetRayDesc().TMax;
        payload.instance = state.GetInstanceID();
        payload.primitive = state.GetPrimitiveIndex();
        state.SetShaderTableIndex(index);
        ReorderThread(state, 0u, 0u);
        HitObject::Invoke(state, payload);
    }
};
[shader("raygeneration")]
void hit_groups_raygen()
{
    uint index = DispatchRaysIndex().x;
    RayDesc ray = { float3(.25, .25, -1), 0, float3(0, 0, 1), 4 };
    Tracer tracer;
    HitObject state;
    tracer.trace(state, ray);
    Payload payload = {};
    tracer.shade(state, index % 3, payload);
    CallShader(index % 2, payload.value);
    result[index * 4] = payload.value;
    result[index * 4 + 1] = asuint(payload.distance);
    result[index * 4 + 2] = payload.instance;
    result[index * 4 + 3] = payload.primitive;
}
[shader("miss")]
void hit_groups_miss(inout Payload payload)
{
    payload.value = 99;
}
[shader("closesthit")]
void hit_groups_first(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.value = 10;
}
[shader("closesthit")]
void hit_groups_second(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.value = 20;
}
[shader("closesthit")]
void hit_groups_third(inout Payload payload, BuiltInTriangleIntersectionAttributes attributes)
{
    payload.value = 30;
}
[shader("callable")]
void hit_groups_add(inout uint value)
{
    value += 3;
}
[shader("callable")]
void hit_groups_scale(inout uint value)
{
    value *= 2;
}
