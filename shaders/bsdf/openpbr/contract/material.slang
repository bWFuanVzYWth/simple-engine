// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/material.slang";

public struct PrimeFullOpenPbrOnb
{
    public var tangent : float3;
    public var bitangent : float3;
    public var normal : float3;
};

public struct PrimeFullOpenPbrWeight
{
    public var base : float;
    public var diffuseRoughness : float;
    public var specular : float;
    public var metalness : float;
    public var subsurface : float;
    public var transmission : float;
    public var coat : float;
    public var fuzz : float;
    public var thinFilm : float;
    public var diffraction : float;
};

public struct PrimeFullOpenPbrGeometry
{
    public var thinWalled : uint;
    public var thickness : float;
    public var onb : PrimeFullOpenPbrOnb;
};

public struct PrimeFullOpenPbrSpecular
{
    public var color : float3;
    public var roughness : float;
    public var roughnessAnisotropy : float;
    public var ior : float;
};

public struct PrimeFullOpenPbrEmission
{
    public var luminance : float3;
};
public struct PrimeFullOpenPbrBase
{
    public var color : float3;
};

public struct PrimeFullOpenPbrSubsurface
{
    public var color : float3;
    public var radius : float3;
    public var scatterAnisotropy : float;
};

public struct PrimeFullOpenPbrTransmission
{
    public var color : float3;
    public var depth : float;
    public var scatter : float3;
    public var scatterAnisotropy : float;
    public var dispersionScale : float;
    public var dispersionAbbeNumber : float;
};

public struct PrimeFullOpenPbrCoat
{
    public var color : float3;
    public var roughness : float;
    public var roughnessAnisotropy : float;
    public var ior : float;
    public var darkening : float;
    public var roughening : float;
    public var onb : PrimeFullOpenPbrOnb;
};

public struct PrimeFullOpenPbrFuzz
{
    public var color : float3;
    public var roughness : float;
};

public struct PrimeFullOpenPbrThinFilm
{
    public var thickness : float;
    public var ior : float;
};

public struct PrimeFullOpenPbrDiffraction
{
    public var color : float3;
    public var thickness : float;
    public var invPitch : float2;
    public var angle : float;
    public var lobeCount : uint;
    public var kind : uint;
};

public struct PrimeFullOpenPbrMaterial
{
    public var weight : PrimeFullOpenPbrWeight;
    public var geometry : PrimeFullOpenPbrGeometry;
    public var specular : PrimeFullOpenPbrSpecular;
    public var emission : PrimeFullOpenPbrEmission;
    public var base : PrimeFullOpenPbrBase;
    public var subsurface : PrimeFullOpenPbrSubsurface;
    public var transmission : PrimeFullOpenPbrTransmission;
    public var coat : PrimeFullOpenPbrCoat;
    public var fuzz : PrimeFullOpenPbrFuzz;
    public var thinFilm : PrimeFullOpenPbrThinFilm;
    public var diffraction : PrimeFullOpenPbrDiffraction;
};
