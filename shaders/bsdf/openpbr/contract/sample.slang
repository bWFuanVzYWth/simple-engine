// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/sample.slang";

import bsdf.openpbr.contract.flags;

namespace openpbr
{
/// 内部局部坐标数值核：响应含cosine、尚未除PDF，proposal尚未做世界坐标sanitize或介质交接。
namespace kernel
{
public struct Response
{
    public var value : float3;
    public var flags : uint;
};

public struct Proposal
{
    public var response : Response;
    public var pdf : float;
    public var eta : float;
    public var wo : float3;
};

public struct PolarizedValue
{
    public var s : float;
    public var p : float;
};
public struct InterfaceResponse
{
    public var reflection : float3;
    public var transmission : float3;
};

public struct RefractResult
{
    public var wo : float3;
    public var relative_ior : float;
    public var valid : uint;
};

public struct Microfacet
{
    public var alpha : float2;
};

public struct SpecularFresnel
{
    public var ior : float;
    public var energy_ior : float;
    public var color : float3;
};

public struct ConductorFresnel
{
    public var f0 : float3;
    // Relative Schlick tint at RoboCute's 1/7 cosine anchor, not absolute F(82 degrees).
    public var f82 : float3;
    public var f90 : float3;
    public var weight : float;
    public var energy_f0 : float3;
};

public func empty_response()->Response
{
    var result : Response;
    result.value = float3(0.0);
    result.flags = LOBE_NONE;
    return result;
}

public func empty_proposal()->Proposal
{
    var result : Proposal;
    result.response = empty_response();
    result.pdf = 0.0;
    result.eta = 1.0;
    result.wo = float3(0.0);
    return result;
}

public func add_response(a: Response, b: Response)->Response
{
    var result : Response;
    result.value = a.value + b.value;
    result.flags = a.flags | b.flags;
    return result;
}

public func scale_response(value: Response, scale: float3)->Response
{
    var value = value;
    value.value *= scale;
    if (all(scale == float3(0.0)))
        value.flags = LOBE_NONE;
    return value;
}

public func scale_response(value: Response, scale: float)->Response
{
    var value = value;
    value.value *= scale;
    if (scale == 0.0)
        value.flags = LOBE_NONE;
    return value;
}
}
}
