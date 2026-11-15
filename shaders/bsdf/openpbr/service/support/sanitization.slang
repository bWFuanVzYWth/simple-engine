// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/service/support/sanitization.slang";

import "bsdf/openpbr/service/contract/sample.slang";
import "bsdf/openpbr/service/contract/albedo.slang";
import "bsdf/openpbr/service/contract/components.slang";

public func primeInvalidBsdfEvaluation()->BsdfEvaluation
{
    var result : BsdfEvaluation;
    result.response = float3(0.0);
    result.pdf = 0.0;
    return result;
}

public func primeInvalidBsdfSample()->BsdfSample
{
    var result : BsdfSample;
    result.direction = float3(0.0);
    result.response = float3(0.0);
    result.pdf = 0.0;
    result.relativeEta = 1.0;
    result.eventFlags = 0;
    return result;
}

public func primeBsdfFinite(value: float)->bool
{
    return !isnan(value) && !isinf(value);
}

public func primeBsdfFinite(value: float3)->bool
{
    return !any(isnan(value)) && !any(isinf(value));
}

public func primeBsdfDirection(value: float3)->bool
{
    if (!primeBsdfFinite(value))
        return false;
    let lengthSquared : float = dot(value, value);
    return primeBsdfFinite(lengthSquared) && lengthSquared > 1.0e-12 &&
           abs(lengthSquared - 1.0) <= 1.0e-3;
}

public func primeSanitizeBsdfEvaluation(evaluation: BsdfEvaluation)->BsdfEvaluation
{
    if (!primeBsdfFinite(evaluation.response) || !primeBsdfFinite(evaluation.pdf) ||
        any(evaluation.response < float3(0.0)) || evaluation.pdf < 0.0)
    {
        return primeInvalidBsdfEvaluation();
    }
    return evaluation;
}

public func primeSanitizeBsdfSample<let UnitEta : bool = false>(sampleValue: BsdfSample)->BsdfSample
{
    if (sampleValue.eventFlags == 0)
    {
        return primeInvalidBsdfSample();
    }
    return primeSanitizeBsdfSampleWithEvents<UnitEta>(sampleValue);
}

/// 调用方已证明 eventFlags 非零；UnitEta=true 另外保证 relativeEta=1。
/// 仍检查方向、响应与 PDF；未证明的 eta 保持原数值检查。
public func primeSanitizeBsdfSampleWithEvents<let UnitEta : bool = false>(sampleValue: BsdfSample)
    ->BsdfSample
{
    if (!primeBsdfDirection(sampleValue.direction) || !primeBsdfFinite(sampleValue.response) ||
        !primeBsdfFinite(sampleValue.pdf) ||
        (!UnitEta && !primeBsdfFinite(sampleValue.relativeEta)) ||
        any(sampleValue.response < float3(0.0)) || !(sampleValue.pdf > 0.0) ||
        (!UnitEta && !(sampleValue.relativeEta > 0.0)))
    {
        return primeInvalidBsdfSample();
    }
    return sampleValue;
}

public func primeSanitizeDenoiseAlbedo(albedo: float3)->float3
{
    return float3(primeBsdfFinite(albedo.x) ? clamp(albedo.x, 0.0, 1.0) : 0.0,
                  primeBsdfFinite(albedo.y) ? clamp(albedo.y, 0.0, 1.0) : 0.0,
                  primeBsdfFinite(albedo.z) ? clamp(albedo.z, 0.0, 1.0) : 0.0);
}

public func primeSanitizeBsdfComponents(components: PrimeBsdfComponents)->PrimeBsdfComponents
{
    var components = components;
    if (!primeBsdfFinite(components.diffuseResponse) ||
        !primeBsdfFinite(components.specularResponse) || !primeBsdfFinite(components.pdf) ||
        any(components.diffuseResponse < float3(0.0)) ||
        any(components.specularResponse < float3(0.0)) || components.pdf < 0.0)
    {
        components.diffuseResponse = float3(0.0);
        components.specularResponse = float3(0.0);
        components.pdf = 0.0;
    }
    return components;
}
