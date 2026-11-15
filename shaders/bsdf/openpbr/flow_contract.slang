/// 资源世代Flow身份与轻量支持查询；数值闭包由composition选择。
module flow_contract;
__exported import bsdf.openpbr.selection;
__exported import bsdf.openpbr.api;
__exported import bsdf.openpbr.specialized;
import bsdf.openpbr.support_composition;
namespace openpbr
{
public func flow_has<let Flow : uint, let Bits : uint>()->bool
{
    return (Flow & Bits) != 0u;
}

/// 整个闭包包含连续测度；光滑塑料的 diffuse 与 delta specular 可以同时存在。
public func flow_continuous<let Flow : uint>()->bool
{
    return flow_has<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                              FLOW_CONTINUOUS_TRANSMISSION>();
}

/// 与is_discrete_only相同的全闭包分类；Flow由资源世代证明，纯测度域不保留数值分类分支。
/// 混合金属的非金属null proposal仍保留原metalness判定，不能把它当成纯金属。
public func is_discrete_only_flow<let Flow : uint>(material: Material, incident: float4,
                                                   transmitted: float4)
    ->bool
{
    return openpbr_composition::DeltaDomain.discrete(material, incident, transmitted);
}

internal func flow_transmission<let Flow : uint>()->bool
{
    return flow_has<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>();
}

internal func require_flow<let Flow : uint>()->void
{
    static_assert((Flow & ~4095u) == 0u, "unknown OpenPBR flow bits");
    static_assert(FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has ==
                      FlowMask<Flow, FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION>.has,
                  "reflection components require their measurement domains");
    static_assert(FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ==
                      FlowMask<Flow, FLOW_DELTA_TRANSMISSION | FLOW_CONTINUOUS_TRANSMISSION>.has,
                  "transmission topology requires its measurement domains");
    static_assert((Flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) !=
                      (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION),
                  "solid and thin are distinct boundary contracts");
    static_assert(
        !FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
            !FlowMask<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONDUCTOR_REFLECTION>.has,
        "opaque and transmission are disjoint in the supported subset");
    static_assert(!FlowMask<Flow, FLOW_METAL_MIX>.has ||
                      (FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has &&
                       !FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has),
                  "metal mixture requires an opaque conductor");
    static_assert(!FlowMask<Flow, FLOW_CONDUCTOR_REFLECTION>.has || !FlowProperties<Flow>.layer ||
                      FlowMask<Flow, FLOW_METAL_MIX>.has,
                  "mixed metalness must retain its proposal control");
    static_assert(!FlowMask<Flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
                      FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      (FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has &&
                       !FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION |
                                           FLOW_DELTA_REFLECTION>.has),
                  "matched transmission is delta only");
    static_assert(!FlowMask<Flow, FLOW_SOLID_TRANSMISSION>.has ||
                      ((!FlowMask<Flow, FLOW_DELTA_REFLECTION>.has ||
                        FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<Flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "solid reflection/transmission domains disagree");
    static_assert(!FlowMask<Flow, FLOW_THIN_TRANSMISSION>.has ||
                      !FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      ((FlowMask<Flow, FLOW_DELTA_REFLECTION>.has ==
                        FlowMask<Flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<Flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<Flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "thin reflection/transmission domains disagree");
}

public func may_evaluate_flow<let Flow : uint>(material: Material, normal: float3, view: float3,
                                               direction: float3, incident: float4,
                                               transmitted: float4)
    ->bool
{
    require_flow<Flow>();
    return openpbr_composition::DirectionSupport.allows(material, normal, view, direction);
}

}
