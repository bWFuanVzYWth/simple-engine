/// 材质专用closest-hit的数值边界；没有路径、几何目录、选灯、可见性或大气状态。
module material_vertex;
import pt.scene;
import pt.integrator;
import pt.transport;
import pt.vertex_call;
import bsdf.openpbr.api;
import bsdf.openpbr.flow;

namespace pt
{
/// 32B无损材质输入。attributes和state低30bit由应用解释，不统一材质来源。
/// normal为几何法线；state高两位分别表示请求散射和存在下一跳。
public struct MaterialContext
{
    public float3 normal;
    public uint4 attributes;
    public uint state;
};

/// 88B有效结果；只包含本次材质求值产生的数值，不保存完整OpenPBR材料。
public struct MaterialVertex
{
    public float4 medium;
    public float3 direction;
    public float3 sun;
    public float3 lamp;
    public float4 response_pdf;
    public float3 emission;
    public float relative_eta;
    public uint events;
};

/// 100B逻辑交接。输入消费后覆盖同一存储，返回仅通过result读取有效输出。
/// 输入geometry/view由调用者保留或实际HitObject提供，不跨边界携带累计路径。
public struct MaterialCall
{
    public MaterialContext context;
    public float4 incident;
    public float3 random;
    public float3 sun;
    public float3 lamp;
    public float4 pdf_distance;

    public static MaterialCall input(MaterialContext context, float4 incident, float3 random,
                                     LightSample sun, LightSample lamp, bool sample,
                                     bool continuation)
    {
        context.state |= (uint(sample) | (uint(continuation) << 1u)) << 30u;
        return { context,        incident,
                 random,         sun.direction,
                 lamp.direction, float4(sun.pdf, sun.distance, lamp.pdf, lamp.distance) };
    }
    [mutating]
    public void store(MaterialVertex value)
    {
        context.normal = value.direction;
        context.attributes = asuint(value.response_pdf);
        context.state = value.events;
        incident = value.medium;
        random = value.emission;
        sun = value.sun;
        lamp = value.lamp;
        pdf_distance.x = value.relative_eta;
    }
    public MaterialVertex result()
    {
        return { incident, context.normal, sun,          lamp, asfloat(context.attributes),
                 random,   pdf_distance.x, context.state };
    }
};

struct MaterialLighting<let Flow : uint> : openpbr::IFlowEvaluationConsumer
{
    float3 normal;
    float3 view;
    float4 incident;
    float4 transmitted;
    DirectSample sun;
    DirectSample lamp;
    bool continuation;
    float3 solar;
    float3 local;
    [mutating]
    void consume(uint direction, openpbr::Evaluation evaluated)
    {
        DirectSample light = direction == 0 ? sun : lamp;
        float3 weight = 0;
        if (light.pdf > 0 && any(evaluated.response > 0))
        {
            float3 extinction = incident.yzw;
            if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0 &&
                dot(view, normal) * dot(light.direction, normal) < 0)
                extinction = transmitted.yzw;
            weight = triple_product(
                evaluated.response, segment_transmittance(extinction, light.distance),
                power_heuristic_over_pdf(light.pdf, continuation ? evaluated.pdf : 0));
        }
        if (direction == 0)
            solar = weight;
        else
            local = weight;
    }
};

/// 应用先将私有context解码为resolve_flow所需Hit；不会读取其他Hit字段。
/// Flow裁掉delta表面的两个光照求值方向及无关透射/反射分支。
public void shade_material<let Flow : uint, Scene : IFlowScene>(Scene provider,
                                                                inout MaterialCall call, Hit hit,
                                                                float3 view)
{
    SurfaceMaterial material = provider.resolve_flow(hit, Flow, call.incident);
    MaterialLighting<Flow> lighting = {};
    lighting.normal = hit.surface.normal;
    lighting.view = view;
    lighting.incident = call.incident;
    lighting.transmitted = material.transmitted;
    lighting.sun = { call.sun, call.pdf_distance.x, call.pdf_distance.y };
    lighting.lamp = { call.lamp, call.pdf_distance.z, call.pdf_distance.w };
    lighting.continuation = (call.context.state & 0x80000000u) != 0;
    uint evaluation = 0;
    static const uint Directions =
        (Flow & (openpbr::FLOW_DIFFUSE | openpbr::FLOW_SUBSURFACE |
                 openpbr::FLOW_CONTINUOUS_REFLECTION | openpbr::FLOW_CONTINUOUS_TRANSMISSION)) != 0
            ? 3u
            : 0u;
    if (Directions != 0)
    {
        if (lighting.sun.pdf > 0 &&
            openpbr::may_evaluate_flow<Flow>(material.pbr, shading_normal(hit, material), view,
                                             call.sun, call.incident, material.transmitted))
            evaluation |= 1u;
        if (lighting.lamp.pdf > 0 &&
            openpbr::may_evaluate_flow<Flow>(material.pbr, shading_normal(hit, material), view,
                                             call.lamp, call.incident, material.transmitted))
            evaluation |= 2u;
    }
    openpbr::Sample sample =
        openpbr::shade_flow_consume<Flow, openpbr::Energy, MaterialLighting<Flow>, Directions>(
            provider.energy(), material.pbr, shading_normal(hit, material), view, call.sun,
            call.lamp, evaluation, call.random, (call.context.state & 0x40000000u) != 0,
            call.incident, material.transmitted, lighting);
    float3 response = sample.response;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0 &&
        (sample.events & openpbr::DISCRETE) != 0 &&
        dot(view, hit.surface.normal) * dot(sample.direction, hit.surface.normal) < 0)
        response *= material.boundary_filter;
    call.store( { sample.medium, sample.direction, lighting.solar, lighting.local,
                  float4(response, sample.pdf), material.emission, sample.relative_eta,
                  sample.events });
}
}
