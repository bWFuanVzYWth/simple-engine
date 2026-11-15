// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/volume.slang";

import "bsdf/openpbr/contract/flags.slang";

public struct PrimeFullOpenPbrVolume
{
    public var extinction : float3;
    public var albedo : float3;
    public var anisotropy : float;
    public var ior : float;
    public var mediumId : uint;
    public var iorEncoding : uint;
};

// A ray carries exactly one resolved medium. Zero is the explicit vacuum endpoint.
public func primeFullOpenPbrVacuumVolume()->PrimeFullOpenPbrVolume
{
    var medium : PrimeFullOpenPbrVolume = {};
    medium.ior = 1.0;
    return medium;
}
