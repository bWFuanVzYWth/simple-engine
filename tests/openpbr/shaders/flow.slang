module specialized_fixture;
import bsdf.openpbr.api;
import bsdf.openpbr.denoise;
import bsdf.openpbr.flow;
import bsdf.openpbr.composition;
import bsdf.openpbr.transmission_composition;
import "bsdf/openpbr/math/frame.slang";
import "bsdf/openpbr/contract/material.slang";

#ifndef OPENPBR_FLOW
#error OPENPBR_FLOW must select one compile-time OpenPBR capability
#endif

struct Case
{
    openpbr::Material material;
    float4 view;
    float4 u;
    float4 incident;
    float4 transmitted;
};
[[vk::binding(0, 0)]]
StructuredBuffer<Case> cases;
[[vk::binding(1, 0)]]
RWStructuredBuffer<float4> output;
[[vk::binding(2, 0)]]
Texture3D<float4> energy_texture;
[[vk::binding(3, 0)]]
SamplerState energy_sampler;

#if !(OPENPBR_FLOW & 48)
void raw_opaque<C : openpbr_composition::IClosure>(Case c, openpbr::Energy energy,
                                                   out openpbr_composition::Proposal proposal,
                                                   out openpbr_composition::Value evaluated)
{
    PrimeFullOpenPbrOnb frame = openpbr::specialized_frame<openpbr::CAP_OPAQUE_DIELECTRIC>(
        float3(0, 0, 1), c.view.xyz, c.incident, c.incident);
    float3 wi = primeFullOpenPbrOnbToLocal(frame, c.view.xyz);
    C.State state = C.prepare(energy, c.material, wi, c.incident.x);
    proposal = {};
    if (c.u.w == 6)
        proposal = openpbr_composition::sample_closure<C>(wi, c.u.xyz, state);
    evaluated = C.evaluate(
        wi, c.u.w == 7 ? primeFullOpenPbrOnbToLocal(frame, c.u.xyz) : proposal.wo, state);
}
#endif

[shader("compute")]
[numthreads(64, 1, 1)]
void main(uint3 id: SV_DispatchThreadID)
{
    uint count, stride;
    cases.GetDimensions(count, stride);
    if (id.x >= count)
        return;
    Case c = cases[id.x];
    openpbr::Energy energy = { energy_texture, energy_sampler };
    // 直接观察组合内核；不能让公开适配器的非有限值拒绝掩盖数值缺陷。
    if (c.u.w == 6 || c.u.w == 7)
    {
        openpbr_composition::Proposal proposal;
        openpbr_composition::Value evaluated;
#if OPENPBR_FLOW & 48
        static const uint Capability =
            (OPENPBR_FLOW & 32) != 0 ? openpbr::CAP_THIN_DIELECTRIC : openpbr::CAP_SOLID_DIELECTRIC;
        PrimeFullOpenPbrOnb frame = openpbr::specialized_frame<Capability>(
            float3(0, 0, 1), c.view.xyz, c.incident, c.transmitted);
        float3 wi = primeFullOpenPbrOnbToLocal(frame, c.view.xyz);
        float ior = c.transmitted.x / c.incident.x;
#if OPENPBR_FLOW & 16
        ior = max(c.incident.x, c.transmitted.x) / min(c.incident.x, c.transmitted.x);
#endif
        openpbr_composition::Transmission.State state = openpbr_composition::Transmission.prepare(
            energy, c.material.base_roughness.w, ior, c.transmitted.yzw * c.material.surface.w, wi);
        proposal = {};
        if (c.u.w == 6)
            proposal = openpbr_composition::Transmission.sample(wi, c.u.xyz, state);
        evaluated = openpbr_composition::Transmission.evaluate(
            wi, c.u.w == 7 ? primeFullOpenPbrOnbToLocal(frame, c.u.xyz) : proposal.wo, state);
#else
#if defined(OPENPBR_RECIPE_VARIANT) && (OPENPBR_VARIANT & 2048) && (OPENPBR_FLOW & 11)
        if (c.material.surface.y == 1)
            raw_opaque<openpbr_composition::Metal>(c, energy, proposal, evaluated);
        else
            raw_opaque<openpbr_composition::Dielectric>(c, energy, proposal, evaluated);
#else
        raw_opaque<openpbr_composition::Opaque>(c, energy, proposal, evaluated);
#endif
#endif
        uint i = id.x * 5;
        output[i] = float4(proposal.wo, proposal.eta);
        output[i + 1] = float4(proposal.throughput.value, proposal.pdf);
        output[i + 2] =
            asfloat(uint4(proposal.throughput.flags, evaluated.throughput.flags, id.x + 1, 0));
        output[i + 3] = float4(evaluated.throughput.value, evaluated.pdf);
        output[i + 4] = 0;
        return;
    }
    if (c.u.w == 5.0)
    {
        openpbr::DenoiseAlbedos value = openpbr::denoise_albedos_flow<OPENPBR_FLOW>(
            energy, c.material, float3(0, 0, 1), c.view.xyz, c.incident, c.transmitted);
        uint i = id.x * 5;
        output[i] = float4(value.diffuse, 0);
        output[i + 1] = float4(value.specular, 0);
        output[i + 2] = asfloat(uint4(0, 0, id.x + 1, 0));
        return;
    }
    uint i = id.x * 5;
    if (c.u.w == 2.0)
    {
        openpbr::Evaluation evaluated = openpbr::evaluate_flow<OPENPBR_FLOW>(
            energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
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
        uint control = uint(c.view.w);
        openpbr::Batch shaded = openpbr::shade_flow<OPENPBR_FLOW>(
            energy, c.material, float3(0, 0, 1), c.view.xyz, float3(0.6, 0, 0.8),
            float3(0.6, 0, -0.8), control & 3u, c.u.xyz, (control & 4u) != 0u, c.incident,
            c.transmitted);
        openpbr::Evaluation evaluated = c.u.w == 3.0 ? shaded.first : shaded.second;
        output[i] = float4(shaded.scattered.direction, shaded.scattered.relative_eta);
        output[i + 1] = float4(shaded.scattered.response, shaded.scattered.pdf);
        output[i + 2] = asfloat(uint4(shaded.scattered.events, 0, id.x + 1, 0));
        output[i + 3] = float4(evaluated.response, evaluated.pdf);
        output[i + 4] = shaded.scattered.medium;
        return;
    }
    openpbr::Sample sampled = openpbr::sample_flow<OPENPBR_FLOW>(
        energy, c.material, float3(0, 0, 1), c.view.xyz, c.u.xyz, c.incident, c.transmitted);
    openpbr::Evaluation evaluated =
        openpbr::evaluate_flow<OPENPBR_FLOW>(energy, c.material, float3(0, 0, 1), c.view.xyz,
                                             sampled.direction, c.incident, c.transmitted);
    output[i] = float4(sampled.direction, sampled.relative_eta);
    output[i + 1] = float4(sampled.response, sampled.pdf);
    output[i + 2] = asfloat(uint4(sampled.events, 0, id.x + 1, 0));
    output[i + 3] = float4(evaluated.response, evaluated.pdf);
    output[i + 4] = sampled.medium;
}
