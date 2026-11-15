// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/math/microfacet_energy.slang";

import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";
import "bsdf/openpbr/math/spectrum.slang";

public interface IPrimeFullOpenPbrTransmissionGgxEnergy
{
    public func sampleLevel(uvw: float3)->float4;
}

func primeFullOpenPbrMicrofacetAlpha2(microfacet: PrimeFullOpenPbrMicrofacet)->float
{
    return 0.5 * primeFullOpenPbrReduceSum(primeFullOpenPbrSquare(microfacet.alpha));
}

public func primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(
    microfacet: PrimeFullOpenPbrMicrofacet, cosineTheta: float, f0: float3, f90: float3)
    ->float3
{
    let x : float = cosineTheta;
    let y2 : float = primeFullOpenPbrMicrofacetAlpha2(microfacet);
    let y : float = sqrt(y2);
    // The same biquadratic, with three independent x chains and no xy temporaries.
    let r0
        : float4 =
              (float4(29.34, 1.424, 28.96, 13.08) * x + float4(-0.6303, -2.323, -1.765, 0.2281)) *
                  x +
              float4(0.1003, 0.9345, 1.0, 1.0);
    let r1 : float4 =
                 (float4(-26.44, 1.436, -36.11, 54.9) * x + float4(-2.038, -3.748, 11.53, -55.83)) *
                     x +
                 float4(9.748, 2.229, 8.263, 15.94);
    let r2 : float4 =
                 (float4(-5.448, 0.6286, 33.37, -285.1) * x + float4(19.99, 0.2913, 15.86, 300.2)) *
                     x +
                 float4(-8.245, -0.7684, -7.507, 41.26);
    let r : float4 = (r2 * y + r1) * y + r0;
    let ab : float2 = clamp(r.xy / r.zw, float2(0.0), float2(1.0));
    return f0 * ab.x + f90 * ab.y;
}

/// Smooth=true 只供已证明 roughness²<1e-4 的 opaque delta specular；微面固定为0，只有 z 被消费。
/// z 的 LUT 坐标在该域固定为首层，仍保留原 base 透过率与 proposal 权重。
public func primeFullOpenPbrMicrofacetDielectricMsCompensation<
    let Smooth : bool = false, TransmissionGgxEnergy>(transmissionGgxEnergy: TransmissionGgxEnergy,
                                                      microfacet: PrimeFullOpenPbrMicrofacet,
                                                      cosineTheta: float, ior: float)
    ->float3
    where TransmissionGgxEnergy : IPrimeFullOpenPbrTransmissionGgxEnergy
{
    if (ior == 1.0)
    {
        return float3(1.0, 1.0, 0.0);
    }

    let qMin = 5.960464477539063e-8;
    let qCritical = 0.00010002000500139488;
    let qMax = 0.999499874937461;
    let nearLog2Scale = 0.09334822745469708;
    let farLog2Min : float = -13.287279491668832;
    let farLog2Scale = 0.04123374454764629;

    let inside : bool = (cosineTheta < 0.0) != (ior < 1.0);
    let mu : float = clamp(abs(cosineTheta), 0.02, 1.0);
    let q : float = clamp(abs((ior - 1.0) / (ior + 1.0)), qMin, qMax);
    let critical : float = min(2.0 * sqrt(q) / (1.0 + q), 0.9999999403953552);
    let nearInside : bool = inside && q <= qCritical;
    let belowCritical : bool = inside && !nearInside && mu < critical;

    let nearU : float = clamp((log2(q) + 24.0) * nearLog2Scale, 0.0, 1.0);
    let nearZ : float = nearU - 0.65 * nearU * (1.0 - nearU) * (2.0 * nearU - 1.0);
    let farU : float = clamp((log2(q / (1.0 - q)) - farLog2Min) * farLog2Scale, 0.0, 1.0);
    let farZ : float = sqrt(farU);
    let exteriorZ : float = q <= qCritical ? (12.0 / 47.0) * nearZ : (12.0 + 35.0 * farZ) / 47.0;
    let logicalZ : float = inside ? (nearInside ? nearZ : farZ) : exteriorZ;
    let zOffset : float = inside ? (nearInside ? 48.0 : (belowCritical ? 111.0 : 63.0)) : 0.0;
    let zExtent : float = nearInside ? 15.0 : 47.0;

    let roughness : float = Smooth ? 0.0 : sqrt(sqrt(primeFullOpenPbrMicrofacetAlpha2(microfacet)));
    let roughnessCoordinate : float =
                                  Smooth ? 0.0 : sqrt(clamp((roughness - 0.01) / 0.99, 0.0, 1.0));
    let linearView : float = (mu - 0.02) / 0.98;
    let belowView : float = (mu - 0.02) / max(critical - 0.02, 1.0e-20);
    let aboveView : float = (mu - critical) / max(1.0 - critical, 1.0e-20);
    var view : float = clamp(belowCritical ? belowView
                                           : (inside && !nearInside ? aboveView : linearView),
                             0.0, 1.0);
    view = view <= 0.5 ? 0.5 * sqrt(2.0 * view) : 1.0 - 0.5 * sqrt(2.0 * (1.0 - view));
    view -= 0.64 * view * (1.0 - view) * (2.0 * view - 1.0);

    let uvw = float3((view * 43.0 + 0.5) / 44.0, (roughnessCoordinate * 31.0 + 0.5) / 32.0,
                     (zOffset + logicalZ * zExtent + 0.5) / 159.0);
    let encoded : float4 = transmissionGgxEnergy.sampleLevel(uvw);

    let a : float = 0.5 * (sqrt(1.0 + primeFullOpenPbrMicrofacetAlpha2(microfacet) *
                                          (1.0 - mu * mu) / (mu * mu)) +
                           1.0);
    let inverseA : float = 1.0 / a;
    let etaOneLog2T
        : float =
              1.0 - 2.0 * a + 0.5 * log2(PRIME_FULL_OPENPBR_PI * a) - 1.45676773e-5 +
              inverseA * (1.80676418e-1 + inverseA * (-1.85532904e-3 - 4.61868116e-3 * inverseA));
    let etaQ0 : float = 3.0e-4 + 6.82190321e-2 * roughness * mu;
    let etaFactor : float = etaQ0 / (q + etaQ0) * etaOneLog2T;
    let qFraction : float = frac(logicalZ * zExtent);
    let transmissionCorrection : float = (2.0 / 3.0) * encoded.w * qFraction * (1.0 - qFraction);
    let scale : float2 =
                    exp2(clamp(float2(encoded.x, encoded.y - etaFactor + transmissionCorrection),
                               float2(-80.0), float2(80.0)));
    let encodedB : float = clamp(encoded.z, 0.0, 1.0);
    let bEndpoint : float = 1.0 - abs(2.0 * encodedB - 1.0);
    let bEdge : float = 0.5 * primeFullOpenPbrSquare(bEndpoint) * sqrt(bEndpoint);
    let helperR : float = encodedB <= 0.5 ? bEdge : 1.0 - bEdge;
    return float3(scale, helperR);
}

public func primeFullOpenPbrMicrofacetDirectionalAlbedoUnity(microfacet: PrimeFullOpenPbrMicrofacet,
                                                             cosineTheta: float)
    ->float
{
    return primeFullOpenPbrMicrofacetDirectionalAlbedoReflection(microfacet, cosineTheta,
                                                                 float3(1.0), float3(1.0))
        .x;
}
