// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/dielectric/contract.slang";

import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/volume.slang";
import "bsdf/openpbr/contract/sample.slang";

public static const let PRIME_FULL_COMPACT_TRANSMISSION_WAVELENGTHS_NM =
    float3(630.0, 532.0, 467.0);

public struct PrimeFullCompactTransmissionState
{
    public var onb : PrimeFullOpenPbrOnb;
    public var geometryThinWalled : uint;
    public var entering : uint;
    public var rayT : float;
    public var originalIor : float;
    public var transmissionColor : float3;
    public var specularMicrofacet : PrimeFullOpenPbrMicrofacet;
    public var specularFresnel : PrimeFullOpenPbrSpecularFresnel;
    public var transmissionMicrofacet : PrimeFullOpenPbrMicrofacet;
    public var transmissionTint : float3;
    public var transmissionVolume : PrimeFullOpenPbrVolume;
    public var transmissionMultipleScattering : float3;
};

public struct PrimeFullCompactTransmissionEvaluation
{
    public var throughput : PrimeFullOpenPbrThroughput;
    public var pdf : float;
};

public func primeFullCompactTransmissionMaterialFrame(w: float3,
                                                      state: PrimeFullCompactTransmissionState)
    ->float3
{
    return state.entering != 0u ? w : float3(-w.x, w.y, -w.z);
}
