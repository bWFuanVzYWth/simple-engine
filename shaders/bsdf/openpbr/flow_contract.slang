/// 资源世代Flow身份与轻量支持查询；数值闭包由composition选择。
module flow_contract;
__exported import bsdf.openpbr.selection;
__exported import bsdf.openpbr.api;
__exported import bsdf.openpbr.specialized;
import bsdf.openpbr.support_composition;
namespace openpbr
{
public func flow_has<let flow : uint, let bits : uint>()->bool
{
    return (flow & bits) != 0u;
}

/// 整个闭包包含连续测度；光滑塑料的 diffuse 与 delta specular 可以同时存在。
public func flow_continuous<let flow : uint>()->bool
{
    return flow_has<flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION |
                              FLOW_CONTINUOUS_TRANSMISSION>();
}

/// 与is_discrete_only相同的全闭包分类；Flow由资源世代证明，纯测度域不保留数值分类分支。
/// 混合金属的非金属null proposal仍保留原metalness判定，不能把它当成纯金属。
public func is_discrete_only_flow<let flow : uint>(material: Material, incident: float4,
                                                   transmitted: float4)
    ->bool
{
    return closure::DeltaDomain.discrete(material, incident, transmitted);
}

internal func flow_transmission<let flow : uint>()->bool
{
    return flow_has<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>();
}

internal func require_flow<let flow : uint>()->void
{
    static_assert((flow & ~4095u) == 0u, "unknown OpenPBR flow bits");
    static_assert(FlowMask<flow, FLOW_DIELECTRIC_REFLECTION | FLOW_CONDUCTOR_REFLECTION>.has ==
                      FlowMask<flow, FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION>.has,
                  "reflection components require their measurement domains");
    static_assert(FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ==
                      FlowMask<flow, FLOW_DELTA_TRANSMISSION | FLOW_CONTINUOUS_TRANSMISSION>.has,
                  "transmission topology requires its measurement domains");
    static_assert((flow & (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION)) !=
                      (FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION),
                  "solid and thin are distinct boundary contracts");
    static_assert(
        !FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
            !FlowMask<flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONDUCTOR_REFLECTION>.has,
        "opaque and transmission are disjoint in the supported subset");
    static_assert(!FlowMask<flow, FLOW_METAL_MIX>.has ||
                      (FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has &&
                       !FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has),
                  "metal mixture requires an opaque conductor");
    static_assert(!FlowMask<flow, FLOW_CONDUCTOR_REFLECTION>.has || !FlowProperties<flow>.layer ||
                      FlowMask<flow, FLOW_METAL_MIX>.has,
                  "mixed metalness must retain its proposal control");
    static_assert(!FlowMask<flow, FLOW_SOLID_TRANSMISSION | FLOW_THIN_TRANSMISSION>.has ||
                      FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      (FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has &&
                       !FlowMask<flow, FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION |
                                           FLOW_DELTA_REFLECTION>.has),
                  "matched transmission is delta only");
    static_assert(!FlowMask<flow, FLOW_SOLID_TRANSMISSION>.has ||
                      ((!FlowMask<flow, FLOW_DELTA_REFLECTION>.has ||
                        FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "solid reflection/transmission domains disagree");
    static_assert(!FlowMask<flow, FLOW_THIN_TRANSMISSION>.has ||
                      !FlowMask<flow, FLOW_DIELECTRIC_REFLECTION>.has ||
                      ((FlowMask<flow, FLOW_DELTA_REFLECTION>.has ==
                        FlowMask<flow, FLOW_DELTA_TRANSMISSION>.has) &&
                       (FlowMask<flow, FLOW_CONTINUOUS_REFLECTION>.has ==
                        FlowMask<flow, FLOW_CONTINUOUS_TRANSMISSION>.has)),
                  "thin reflection/transmission domains disagree");
}

public func may_evaluate_flow<let flow : uint>(material: Material, normal: float3, view: float3,
                                               direction: float3, incident: float4,
                                               transmitted: float4)
    ->bool
{
    require_flow<flow>();
    return closure::DirectionSupport.allows(material, normal, view, direction);
}

}
