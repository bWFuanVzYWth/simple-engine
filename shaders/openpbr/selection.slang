/// 配方在构建边界绑定值；内部类型选择保持编译期求值，不引入联合状态。
module selection;

namespace openpbr {
namespace closure {
public struct TypeSelect<let condition : bool, TrueType, FalseType> {};
public extension<TrueType, FalseType> TypeSelect<true, TrueType, FalseType> {
    public typedef TrueType Type;
};
public extension<TrueType, FalseType> TypeSelect<false, TrueType, FalseType> {
    public typedef FalseType Type;
};

/// 与Rust Recipe::weight_bits保持相同的二值权重位。
public static const let variant_metal_switch : uint = 1u << 11;
public static const let variant_subsurface_switch : uint = 1u << 12;

#if defined(OPENPBR_FLOW)
public static const let selected_flow : uint = OPENPBR_FLOW;
public static const let has_selected_flow = true;
#elif defined(GAME_FLOW)
public static const let selected_flow : uint = GAME_FLOW;
public static const let has_selected_flow = true;
#else
public static const let selected_flow : uint = 0;
public static const let has_selected_flow = false;
#endif

#ifdef OPENPBR_RECIPE_VARIANT
public static const let selected_variant : uint = OPENPBR_VARIANT;
#else
public static const let selected_variant : uint = 0;
#endif
}
}

namespace openpbr {
public static const let FLOW_DIFFUSE : uint = 1u << 0;
public static const let FLOW_DIELECTRIC_REFLECTION : uint = 1u << 1;
public static const let FLOW_CONDUCTOR_REFLECTION : uint = 1u << 2;
public static const let FLOW_SUBSURFACE : uint = 1u << 3;
public static const let FLOW_SOLID_TRANSMISSION : uint = 1u << 4;
public static const let FLOW_THIN_TRANSMISSION : uint = 1u << 5;
public static const let FLOW_DELTA_REFLECTION : uint = 1u << 6;
public static const let FLOW_CONTINUOUS_REFLECTION : uint = 1u << 7;
public static const let FLOW_DELTA_TRANSMISSION : uint = 1u << 8;
public static const let FLOW_CONTINUOUS_TRANSMISSION : uint = 1u << 9;
public static const let FLOW_EMISSION : uint = 1u << 10;
/// 金属选择权重可小于1；即使非金属端被证明为吸收，也必须保留其 null proposal。
public static const let FLOW_METAL_MIX : uint = 1u << 11;

/// 泛型值直接形成常量表达式，供 static_assert 与数值部件共同消费。
public struct FlowMask<let flow : uint, let bits : uint> {
    public static const let has : bool = (flow & bits) != 0u;
};
public struct FlowProperties<let flow : uint> {
    public static const let base : bool = FlowMask<flow, FLOW_DIFFUSE | FLOW_SUBSURFACE>.has;
    public static const let layer : bool =
        FlowMask<flow, FLOW_DIELECTRIC_REFLECTION | FLOW_DIFFUSE | FLOW_SUBSURFACE>.has;
    public static const let continuous: bool = FlowMask<flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION>.has;
};
}
