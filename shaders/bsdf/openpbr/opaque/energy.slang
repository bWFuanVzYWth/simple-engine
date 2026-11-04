// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/energy.slang";

import "bsdf/openpbr/opaque/state.slang";
import "bsdf/openpbr/opaque/lobes.slang";
import "bsdf/openpbr/contract/flags.slang";

public PrimeFullCompactOpaqueEnergy primeFullCompactOpaqueEnergy(float3 wi,
                                                                 PrimeFullCompactOpaqueState state)
{
    PrimeFullCompactOpaqueEnergy result;
    result.diffuse = float3(0.0);
    result.specular = float3(0.0);
    if (state.kind == PRIME_FULL_COMPACT_OPAQUE_CONDUCTOR)
    {
        result.specular = primeFullCompactConductorEnergy(wi, state, PRIME_FULL_OPENPBR_FLAG_ALL);
        return result;
    }
    float3 base = state.kind == PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE
                      ? primeFullCompactMixedDiffuseEnergy(state, PRIME_FULL_OPENPBR_FLAG_DIFFUSE)
                  : state.kind == PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE
                      ? primeFullCompactSubsurfaceEnergy(state, PRIME_FULL_OPENPBR_FLAG_DIFFUSE)
                      : primeFullCompactLambertEnergy(wi, state, PRIME_FULL_OPENPBR_FLAG_DIFFUSE);
    result.diffuse = base * state.layer.transmittanceIn;
    result.specular = primeFullCompactDielectricEnergy(
        wi, state, PRIME_FULL_OPENPBR_FLAG_SPECULAR | PRIME_FULL_OPENPBR_FLAG_DELTA);
    if (state.metalness > 0.0)
    {
        result.diffuse *= 1.0 - state.metalness;
        result.specular =
            lerp(result.specular,
                 primeFullCompactConductorEnergy(wi, state, PRIME_FULL_OPENPBR_FLAG_ALL),
                 state.metalness);
    }
    return result;
}
