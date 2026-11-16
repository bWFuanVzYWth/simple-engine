// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/microfacet_measure.slang";

import bsdf.openpbr.contract.material;
import bsdf.openpbr.contract.sample;
import bsdf.openpbr.math.spectrum;

namespace openpbr
{
namespace kernel
{
public func microfacet_effectively_smooth(microfacet: Microfacet)->bool
{
    return reduce_max(microfacet.alpha) < 1.0e-4;
}
}
}
