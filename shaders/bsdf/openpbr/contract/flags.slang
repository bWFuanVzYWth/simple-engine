// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "bsdf/openpbr/contract/flags.slang";

public static const let PRIME_FULL_OPENPBR_FLAG_NONE = 0u;
public static const let PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION : uint = 1u << 0u;
public static const let PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION : uint = 1u << 1u;
public static const let PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION : uint = 1u << 2u;
public static const let PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION : uint = 1u << 3u;
public static const let PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION : uint = 1u << 4u;
public static const let PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION : uint = 1u << 5u;

public static const let PRIME_FULL_OPENPBR_FLAG_DIFFUSE
    : uint =
          PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION | PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION;
public static const let PRIME_FULL_OPENPBR_FLAG_SPECULAR
    : uint = PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION |
             PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION;
public static const let PRIME_FULL_OPENPBR_FLAG_DELTA
    : uint = PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION | PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION;
public static const let PRIME_FULL_OPENPBR_FLAG_REFLECTION
    : uint = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_REFLECTION |
             PRIME_FULL_OPENPBR_FLAG_SPECULAR_REFLECTION | PRIME_FULL_OPENPBR_FLAG_DELTA_REFLECTION;
public static const let PRIME_FULL_OPENPBR_FLAG_TRANSMISSION
    : uint = PRIME_FULL_OPENPBR_FLAG_DIFFUSE_TRANSMISSION |
             PRIME_FULL_OPENPBR_FLAG_SPECULAR_TRANSMISSION |
             PRIME_FULL_OPENPBR_FLAG_DELTA_TRANSMISSION;
public static const let PRIME_FULL_OPENPBR_FLAG_ALL
    : uint = PRIME_FULL_OPENPBR_FLAG_REFLECTION | PRIME_FULL_OPENPBR_FLAG_TRANSMISSION;

public static const let PRIME_FULL_OPENPBR_DIFFRACTION_RECTANGULAR = 1u;

public func primeFullOpenPbrIsReflective(flags: uint)->bool
{
    return (flags & PRIME_FULL_OPENPBR_FLAG_REFLECTION) != 0u;
}

public func primeFullOpenPbrIsTransmissive(flags: uint)->bool
{
    return (flags & PRIME_FULL_OPENPBR_FLAG_TRANSMISSION) != 0u;
}

public func primeFullOpenPbrIsDiffuse(flags: uint)->bool
{
    return (flags & PRIME_FULL_OPENPBR_FLAG_DIFFUSE) != 0u;
}

public func primeFullOpenPbrIsSpecular(flags: uint)->bool
{
    return (flags & PRIME_FULL_OPENPBR_FLAG_SPECULAR) != 0u;
}

public func primeFullOpenPbrIsDelta(flags: uint)->bool
{
    return (flags & PRIME_FULL_OPENPBR_FLAG_DELTA) != 0u;
}

public func primeFullOpenPbrIsNonDelta(flags: uint)->bool
{
    return (flags & (PRIME_FULL_OPENPBR_FLAG_DIFFUSE | PRIME_FULL_OPENPBR_FLAG_SPECULAR)) != 0u;
}
