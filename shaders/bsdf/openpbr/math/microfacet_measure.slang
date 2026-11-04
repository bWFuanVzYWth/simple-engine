// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/microfacet_measure.slang";

import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";

public bool primeFullOpenPbrMicrofacetEffectivelySmooth(PrimeFullOpenPbrMicrofacet microfacet)
{
    return primeFullOpenPbrReduceMax(microfacet.alpha) < 1.0e-4;
}
