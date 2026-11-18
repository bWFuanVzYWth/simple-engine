[[vk::binding(0, 0)]]
var scene : RaytracingAccelerationStructure;
[[vk::binding(1, 0)]]
var result : RWStructuredBuffer<uint>;
struct Payload {
    var value : uint;
    var distance : float;
    var instance : uint;
    var primitive : uint;
};
interface ITracer<State> {
    func trace(inout state: State, ray: RayDesc)->void;
    func shade(inout state: State, index: uint, inout payload_value: Payload)->void;
};
struct Tracer : ITracer<HitObject> {
    func trace(inout state: HitObject, ray: RayDesc)->void {
        var unused : uint = 0;
        state = HitObject::TraceRay(scene, RAY_FLAG_NONE, 0xff, 0, 1, 0, ray, unused);
    }
    func shade(inout state: HitObject, index: uint, inout payload_value: Payload)->void {
        payload_value.distance = state.GetRayDesc().TMax;
        payload_value.instance = state.GetInstanceID();
        payload_value.primitive = state.GetPrimitiveIndex();
        state.SetShaderTableIndex(index);
        ReorderThread(state, 0u, 0u);
        HitObject::Invoke(state, payload_value);
    }
};
[shader("raygeneration")]
func hit_groups_raygen()->void {
    var index : uint = DispatchRaysIndex().x;
    var ray : RayDesc = { float3(.25, .25, -1), 0, float3(0, 0, 1), 4 };
    var tracer : Tracer;
    var state : HitObject;
    tracer.trace(state, ray);
    var payload : Payload = {};
    tracer.shade(state, index % 3, payload);
    CallShader(index % 2, payload.value);
    result[index * 4] = payload.value;
    result[index * 4 + 1] = asuint(payload.distance);
    result[index * 4 + 2] = payload.instance;
    result[index * 4 + 3] = payload.primitive;
}
[shader("miss")]
func hit_groups_miss(inout payload_value: Payload)->void {
    payload_value.value = 99;
}
[shader("closesthit")]
func hit_groups_first(inout payload_value: Payload,
                      attributes: BuiltInTriangleIntersectionAttributes)
    ->void {
    payload_value.value = 10;
}
[shader("closesthit")]
func hit_groups_second(inout payload_value: Payload,
                       attributes: BuiltInTriangleIntersectionAttributes)
    ->void {
    payload_value.value = 20;
}
[shader("closesthit")]
func hit_groups_third(inout payload_value: Payload,
                      attributes: BuiltInTriangleIntersectionAttributes)
    ->void {
    payload_value.value = 30;
}
[shader("callable")]
func hit_groups_add(inout value: uint)->void {
    value += 3;
}
[shader("callable")]
func hit_groups_scale(inout value: uint)->void {
    value *= 2;
}
