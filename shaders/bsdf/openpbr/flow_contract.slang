/// 资源世代Flow身份与轻量支持查询；数值闭包由composition选择。
module flow_contract;
__exported import bsdf.openpbr.api;
__exported import bsdf.openpbr.specialized;
import bsdf.openpbr.support_composition;
namespace openpbr
{
public static const uint FLOW_DIFFUSE = 1u << 0;
public static const uint FLOW_DIELECTRIC_REFLECTION = 1u << 1;
public static const uint FLOW_CONDUCTOR_REFLECTION = 1u << 2;
public static const uint FLOW_SUBSURFACE = 1u << 3;
public static const uint FLOW_SOLID_TRANSMISSION = 1u << 4;
public static const uint FLOW_THIN_TRANSMISSION = 1u << 5;
public static const uint FLOW_DELTA_REFLECTION = 1u << 6;
public static const uint FLOW_CONTINUOUS_REFLECTION = 1u << 7;
public static const uint FLOW_DELTA_TRANSMISSION = 1u << 8;
public static const uint FLOW_CONTINUOUS_TRANSMISSION = 1u << 9;
public static const uint FLOW_EMISSION = 1u << 10;
/// 金属选择权重可小于1；即使非金属端被证明为吸收，也必须保留其 null proposal。
public static const uint FLOW_METAL_MIX = 1u << 11;

public bool flow_has<let Flow : uint, let Bits : uint>()
{
    return (Flow & Bits) != 0u;
}

/// 整个闭包包含连续测度；光滑塑料的 diffuse 与 delta specular 可以同时存在。
public bool flow_continuous<let Flow : uint>()
{
    return flow_has<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                              FLOW_CONTINUOUS_TRANSMISSION>();
}

/// 与is_discrete_only相同的全闭包分类；Flow由资源世代证明，纯测度域不保留数值分类分支。
/// 混合金属的非金属null proposal仍保留原metalness判定，不能把它当成纯金属。
public bool is_discrete_only_flow<let Flow : uint>(Material material, float4 incident,
                                                   float4 transmitted)
{
    return openpbr_composition::DeltaDomain.discrete(material, incident, transmitted);
}

internal bool flow_transmission<let Flow : uint>()
{
    return flow_has<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>();
}

#define FLOW_HAS(Key, Bits) (((Key) & (Bits)) != 0u)
#define FLOW_BASE(Key) FLOW_HAS(Key, FLOW_DIFFUSE | FLOW_SUBSURFACE)
#define FLOW_LAYER(Key) FLOW_HAS(Key, FLOW_DIELECTRIC_REFLECTION | FLOW_DIFFUSE | FLOW_SUBSURFACE)
#define FLOW_CONTINUOUS(Key)                                                                       \
    FLOW_HAS(Key, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION)
internal void require_flow<let Flow : uint>()
{
    static_assert((Flow & ~4095u) == 0u, "unknown OpenPBR flow bits");
    static_assert(FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION) ==
                      FLOW_HAS(Flow, FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION),
                  "reflection components require their measurement domains");
    static_assert(FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION) ==
                      FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION | FLOW_CONTINUOUS_TRANSMISSION),
                  "transmission topology requires its measurement domains");
    static_assert((Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) !=
                      (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION),
                  "solid and thin are distinct boundary contracts");
    static_assert(!FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION) ||
                      !FLOW_HAS(Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONDUCTOR_REFLECTION),
                  "opaque and transmission are disjoint in the supported subset");
    static_assert(!FLOW_HAS(Flow, FLOW_METAL_MIX) ||
                      (FLOW_HAS(Flow, FLOW_CONDUCTOR_REFLECTION) &&
                       !FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)),
                  "metal mixture requires an opaque conductor");
    static_assert(!FLOW_HAS(Flow, FLOW_CONDUCTOR_REFLECTION) || !FLOW_LAYER(Flow) ||
                      FLOW_HAS(Flow, FLOW_METAL_MIX),
                  "mixed metalness must retain its proposal control");
    static_assert(!FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION) ||
                      FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) ||
                      (FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION) &&
                       !FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION |
                                           FLOW_DELTA_REFLECTION)),
                  "matched transmission is delta only");
    static_assert(
        !FLOW_HAS(Flow, FLOW_SOLID_TRANSMISSION) ||
            ((!FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) || FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION)) &&
             (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) ==
              FLOW_HAS(Flow, FLOW_CONTINUOUS_TRANSMISSION))),
        "solid reflection/transmission domains disagree");
    static_assert(
        !FLOW_HAS(Flow, FLOW_THIN_TRANSMISSION) || !FLOW_HAS(Flow, FLOW_DIELECTRIC_REFLECTION) ||
            ((FLOW_HAS(Flow, FLOW_DELTA_REFLECTION) == FLOW_HAS(Flow, FLOW_DELTA_TRANSMISSION)) &&
             (FLOW_HAS(Flow, FLOW_CONTINUOUS_REFLECTION) ==
              FLOW_HAS(Flow, FLOW_CONTINUOUS_TRANSMISSION))),
        "thin reflection/transmission domains disagree");
}

public bool may_evaluate_flow<let Flow : uint>(Material material, float3 normal, float3 view,
                                               float3 direction, float4 incident,
                                               float4 transmitted)
{
    require_flow<Flow>();
    return openpbr_composition::DirectionSupport.allows(material, normal, view, direction);
}

}
