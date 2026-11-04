// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/volume.slang";

import "bsdf/openpbr/contract/flags.slang";

public struct PrimeFullOpenPbrVolume
{
    public float3 extinction;
    public float3 albedo;
    public float anisotropy;
    public float ior;
    public uint mediumId;
    public uint iorEncoding;
};

// A ray carries exactly one resolved medium. Zero is the explicit vacuum endpoint.
public PrimeFullOpenPbrVolume primeFullOpenPbrVacuumVolume()
{
    PrimeFullOpenPbrVolume medium = {};
    medium.ior = 1.0;
    return medium;
}
