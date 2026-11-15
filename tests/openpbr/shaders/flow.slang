module specialized_fixture;
import bsdf.openpbr.api;
import bsdf.openpbr.denoise;
import bsdf.openpbr.flow;
import bsdf.openpbr.composition;
import bsdf.openpbr.selection;
import bsdf.openpbr.transmission_composition;
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/contract/material.slang";

#ifndef OPENPBR_FLOW
#error OPENPBR_FLOW must select one compile-time OpenPBR capability
#endif

struct Case
{
    var material : openpbr::Material;
    var view : float4;
    var u : float4;
    var incident : float4;
    var transmitted : float4;
};
[[vk::binding(0, 0)]]
var cases : StructuredBuffer<Case>;
[[vk::binding(1, 0)]]
var output : RWStructuredBuffer<float4>;
[[vk::binding(2, 0)]]
var energy_texture : Texture3D<float4>;
[[vk::binding(3, 0)]]
var energy_sampler : SamplerState;

func raw_opaque<C : openpbr_composition::IClosure>(c: Case, energy: openpbr::Energy,
                                                   out proposal: openpbr_composition::Proposal,
                                                   out evaluated: openpbr_composition::Value)
    ->void
{
    let frame : PrimeFullOpenPbrOnb = openpbr::specialized_frame<openpbr::CAP_OPAQUE_DIELECTRIC>(
                    float3(0, 0, 1), c.view.xyz, c.incident, c.incident);
    let wi : float3 = primeFullOpenPbrOnbToLocal(frame, c.view.xyz);
    let state : C.State = C.prepare(energy, c.material, wi, c.incident.x);
    proposal = {};
    if (c.u.w == 6)
        proposal = openpbr_composition::sample_closure<C>(wi, c.u.xyz, state);
    evaluated = C.evaluate(
        wi, c.u.w == 7 ? primeFullOpenPbrOnbToLocal(frame, c.u.xyz) : proposal.wo, state);
}

[shader("compute")]
[numthreads(64, 1, 1)]
func main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    cases.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c : Case = cases[id.x];
    let energy : openpbr::Energy = { energy_texture, energy_sampler };
    // 直接观察组合内核；不能让公开适配器的非有限值拒绝掩盖数值缺陷。
    if (c.u.w == 6 || c.u.w == 7)
    {
        var proposal : openpbr_composition::Proposal;
        var evaluated : openpbr_composition::Value;
        if ((OPENPBR_FLOW & 48) != 0)
        {
            static const let Capability : uint = (OPENPBR_FLOW & 32) != 0
                                                     ? openpbr::CAP_THIN_DIELECTRIC
                                                     : openpbr::CAP_SOLID_DIELECTRIC;
            let frame : PrimeFullOpenPbrOnb = openpbr::specialized_frame<Capability>(
                            float3(0, 0, 1), c.view.xyz, c.incident, c.transmitted);
            let wi : float3 = primeFullOpenPbrOnbToLocal(frame, c.view.xyz);
            var ior : float = c.transmitted.x / c.incident.x;
            if ((OPENPBR_FLOW & 16) != 0)
            {
                ior = max(c.incident.x, c.transmitted.x) / min(c.incident.x, c.transmitted.x);
            }
            let state : openpbr_composition::Transmission.State =
                openpbr_composition::Transmission.prepare(energy, c.material.base_roughness.w, ior,
                                                          c.transmitted.yzw * c.material.surface.w,
                                                          wi);
            proposal = {};
            if (c.u.w == 6)
                proposal = openpbr_composition::Transmission.sample(wi, c.u.xyz, state);
            evaluated = openpbr_composition::Transmission.evaluate(
                wi, c.u.w == 7 ? primeFullOpenPbrOnbToLocal(frame, c.u.xyz) : proposal.wo, state);
        }
        else
        {
            if ((openpbr_composition::SelectedVariant & 2048) != 0 && (OPENPBR_FLOW & 11) != 0)
            {
                if (c.material.surface.y == 1)
                    raw_opaque<openpbr_composition::Metal>(c, energy, proposal, evaluated);
                else
                    raw_opaque<openpbr_composition::Dielectric>(c, energy, proposal, evaluated);
            }
            else
            {
                raw_opaque<openpbr_composition::Opaque>(c, energy, proposal, evaluated);
            }
        }
        let i : uint = id.x * 5;
        output[i] = float4(proposal.wo, proposal.eta);
        output[i + 1] = float4(proposal.throughput.value, proposal.pdf);
        output[i + 2] =
            asfloat(uint4(proposal.throughput.flags, evaluated.throughput.flags, id.x + 1, 0));
        output[i + 3] = float4(evaluated.throughput.value, evaluated.pdf);
        output[i + 4] = 0;
        return;
    }
    let i : uint = id.x * 5;
    if (c.u.w == 2.0)
    {
        let evaluated : openpbr::Evaluation = openpbr::evaluate_flow<OPENPBR_FLOW>(
                            energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident,
                            c.transmitted);
        output[i + 2] = asfloat(
            uint4(0,
                  openpbr::may_evaluate_flow<OPENPBR_FLOW>(c.material, float3(0, 0, 1), c.view.xyz,
                                                           c.u.xyz, c.incident, c.transmitted),
                  id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        return;
    }
    if (c.u.w == 3.0 || c.u.w == 4.0)
    {
        let control = uint(c.view.w);
        let shaded : openpbr::Batch = openpbr::shade_flow<OPENPBR_FLOW>(
                         energy, c.material, float3(0, 0, 1), c.view.xyz, float3(0.6, 0, 0.8),
                         float3(0.6, 0, -0.8), control & 3u, c.u.xyz, (control & 4u) != 0u,
                         c.incident, c.transmitted);
        let evaluated : openpbr::Evaluation = c.u.w == 3.0 ? shaded.first : shaded.second;
        output[i] = float4(shaded.scattered.direction, shaded.scattered.relative_eta);
        output[i + 1] = float4(shaded.scattered.response, shaded.scattered.pdf);
        output[i + 2] = asfloat(uint4(shaded.scattered.events, 0, id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        output[i + 4] = shaded.scattered.medium;
        return;
    }
    let sampled
        : openpbr::Sample = openpbr::sample_flow<OPENPBR_FLOW>(
              energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
    let evaluated
        : openpbr::Evaluation =
              openpbr::evaluate_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                                   sampled.direction, c.incident, c.transmitted);
    output[i] = float4(sampled.direction, sampled.relative_eta);
    output[i + 1] = float4(sampled.response, sampled.pdf);
    output[i + 2] = asfloat(uint4(sampled.events, 0, id.x + 1, 0));
    output[i + 3] = float4(evaluated.response, evaluated.pdf);
    output[i + 4] = sampled.medium;
}

// 导引单独编译；其 LUT 资源不属于纯闭包入口的绑定契约。
[shader("compute")]
[numthreads(64, 1, 1)]
func guide_main(uint3 id: SV_DispatchThreadID)->void
{
    var count : uint;
    var stride : uint;
    cases.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    let c = cases[id.x];
    let energy : openpbr::Energy = { energy_texture, energy_sampler };

    let value : openpbr::DenoiseAlbedos = openpbr::denoise_albedos_flow<OPENPBR_FLOW>(
                    energy, c.material, float3(0, 0, 1), c.view.xyz, c.incident, c.transmitted);
    let i : uint = id.x * 5;
    output[i] = float4(value.diffuse, 0);
    output[i + 1] = float4(value.specular, 0);
    output[i + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
    return;
}
