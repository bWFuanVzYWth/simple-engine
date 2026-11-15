// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/opaque/state.slang";

import "bsdf/openpbr/contract/material.slang";
import "bsdf/openpbr/contract/sample.slang";

public static const let PRIME_FULL_COMPACT_OPAQUE_DIELECTRIC = 0u;
public static const let PRIME_FULL_COMPACT_OPAQUE_CONDUCTOR = 1u;
public static const let PRIME_FULL_COMPACT_OPAQUE_SUBSURFACE = 2u;
public static const let PRIME_FULL_COMPACT_OPAQUE_MIXED_SUBSURFACE = 3u;

public struct PrimeFullCompactMixState
{
    public var firstSampleWeight : float;
    public var secondSampleWeight : float;
};

public struct PrimeFullCompactLayerState
{
    public var transmittanceIn : float3;
    public var baseSampleWeight : float;
    public var specularSampleWeight : float;
};

public struct PrimeFullCompactOpaqueState
{
    public var onb : PrimeFullOpenPbrOnb;
    public var kind : uint;
    public var metalness : float;
    public var baseColor : float3;
    public var subsurfaceWeight : float;
    public var subsurfaceColor : float3;
    public var subsurfaceAnisotropy : float;
    public var microfacet : PrimeFullOpenPbrMicrofacet;
    public var dielectricFresnel : PrimeFullOpenPbrSpecularFresnel;
    public var dielectricMultipleScattering : float3;
    public var conductorFresnel : PrimeFullOpenPbrConductorFresnel;
    public var conductorEss : float;
    public var diffuseMix : PrimeFullCompactMixState;
    public var layer : PrimeFullCompactLayerState;
};

public struct PrimeFullCompactOpaqueEvaluation
{
    public var throughput : PrimeFullOpenPbrThroughput;
    public var pdf : float;
};

public struct PrimeFullCompactOpaqueComponents
{
    public var diffuse : float3;
    public var specular : float3;
    public var pdf : float;
};

public struct PrimeFullCompactOpaqueEnergy
{
    public var diffuse : float3;
    public var specular : float3;
};
