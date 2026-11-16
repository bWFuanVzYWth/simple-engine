// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/volume_sample.slang";

import bsdf.openpbr.contract.sample;
import bsdf.openpbr.contract.volume;

namespace openpbr
{
namespace kernel
{
public struct SampleResult
{
    public var proposal : Proposal;
    public var medium : Volume;
    public var ray_t : float;
};
}
}
