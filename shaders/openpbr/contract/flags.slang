// RoboCute 来源与 Apache-2.0 许可见 crates/openpbr/assets/NOTICE.md。

#language slang 2026
module "openpbr/contract/flags.slang";

namespace openpbr {
namespace kernel {
public static const let LOBE_NONE = 0u;
public static const let LOBE_DIFFUSE_REFLECTION : uint = 1u << 0u;
public static const let LOBE_DIFFUSE_TRANSMISSION : uint = 1u << 1u;
public static const let LOBE_SPECULAR_REFLECTION : uint = 1u << 2u;
public static const let LOBE_SPECULAR_TRANSMISSION : uint = 1u << 3u;
public static const let LOBE_DELTA_REFLECTION : uint = 1u << 4u;
public static const let LOBE_DELTA_TRANSMISSION : uint = 1u << 5u;

public static const let LOBE_DIFFUSE : uint = LOBE_DIFFUSE_REFLECTION | LOBE_DIFFUSE_TRANSMISSION;
public static const let LOBE_SPECULAR : uint =
                                            LOBE_SPECULAR_REFLECTION | LOBE_SPECULAR_TRANSMISSION;
public static const let LOBE_DELTA : uint = LOBE_DELTA_REFLECTION | LOBE_DELTA_TRANSMISSION;
public static const let LOBE_REFLECTION
    : uint = LOBE_DIFFUSE_REFLECTION | LOBE_SPECULAR_REFLECTION | LOBE_DELTA_REFLECTION;
public static const let LOBE_TRANSMISSION
    : uint = LOBE_DIFFUSE_TRANSMISSION | LOBE_SPECULAR_TRANSMISSION | LOBE_DELTA_TRANSMISSION;
public static const let LOBE_ALL : uint = LOBE_REFLECTION | LOBE_TRANSMISSION;

public static const let DIFFRACTION_RECTANGULAR = 1u;

public func is_reflective(flags: uint)->bool {
    return (flags & LOBE_REFLECTION) != 0u;
}

public func is_transmissive(flags: uint)->bool {
    return (flags & LOBE_TRANSMISSION) != 0u;
}

public func is_diffuse(flags: uint)->bool {
    return (flags & LOBE_DIFFUSE) != 0u;
}

public func is_specular(flags: uint)->bool {
    return (flags & LOBE_SPECULAR) != 0u;
}

public func is_delta(flags: uint)->bool {
    return (flags & LOBE_DELTA) != 0u;
}

public func is_non_delta(flags: uint)->bool {
    return (flags & (LOBE_DIFFUSE | LOBE_SPECULAR)) != 0u;
}
}
}
