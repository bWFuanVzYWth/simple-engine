// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/volume_sample.slang";

import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/contract/volume.slang";

public struct PrimeFullOpenPbrSampleResult
{
    public PrimeFullOpenPbrSample bsdfSample;
    public PrimeFullOpenPbrVolume medium;
    public float rayT;
};
