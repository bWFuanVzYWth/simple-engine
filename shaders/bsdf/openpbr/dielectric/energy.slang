// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/energy.slang";

import bsdf.openpbr.dielectric.contract;
import bsdf.openpbr.math.microfacet_measure;
import bsdf.openpbr.math.microfacet_energy;
import bsdf.openpbr.contract.flags;
import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.sample;

namespace openpbr
{
namespace kernel
{
public func transmission_energy<EnergyLut>(energy_lut: EnergyLut, wi: float3,
                                           state: TransmissionState, sampling_flags: uint)
    ->float3
    where EnergyLut : openpbr::IEnergyLut
{
    if (!microfacet_effectively_smooth(state.specular_microfacet) &&
        state.specular_fresnel.ior != 1.0)
    {
        if (!is_specular(sampling_flags))
        {
            return float3(0.0);
        }
    }
    else if (!is_delta(sampling_flags))
    {
        return float3(0.0);
    }
    let helper_r : float = state.geometry_thin_walled != 0u
                               ? dielectric_energy(energy_lut, state.specular_microfacet, abs(wi.z),
                                                   state.specular_fresnel.ior)
                                     .z
                               : state.transmission_multiple_scattering.z;
    let reflected : float = is_reflective(sampling_flags) ? helper_r : 0.0;
    let transmitted : float = is_transmissive(sampling_flags) ? 1.0 - helper_r : 0.0;
    return reflected * state.specular_fresnel.color + transmitted * state.transmission_tint;
}
}
}
