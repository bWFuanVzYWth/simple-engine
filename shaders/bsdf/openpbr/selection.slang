/// 配方在构建边界绑定值；内部类型选择保持编译期求值，不引入联合状态。
module selection;

namespace openpbr_composition
{
public struct TypeSelect<let Condition : bool, T, F>
{
};
public extension<T, F> TypeSelect<true, T, F>
{
    public typedef T Type;
};
public extension<T, F> TypeSelect<false, T, F>
{
    public typedef F Type;
};

#if defined(OPENPBR_FLOW)
public static const let SelectedFlow : uint = OPENPBR_FLOW;
public static const let HasSelectedFlow = true;
#elif defined(GAME_FLOW)
public static const let SelectedFlow : uint = GAME_FLOW;
public static const let HasSelectedFlow = true;
#else
public static const let SelectedFlow : uint = 0;
public static const let HasSelectedFlow = false;
#endif

#ifdef OPENPBR_RECIPE_VARIANT
public static const let SelectedVariant : uint = OPENPBR_VARIANT;
#else
public static const let SelectedVariant : uint = 0;
#endif
}

namespace openpbr
{
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
public struct FlowMask<let Flow : uint, let Bits : uint>
{
    public static const let has : bool = (Flow & Bits) != 0u;
};
public struct FlowProperties<let Flow : uint>
{
    public static const let base : bool = FlowMask<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE>.has;
    public static const let layer : bool =
        FlowMask<Flow, FLOW_DIELECTRIC_REFLECTION | FLOW_DIFFUSE | FLOW_SUBSURFACE>.has;
    public static const let continuous: bool = FlowMask<Flow, FLOW_DIFFUSE | FLOW_SUBSURFACE | FLOW_CONTINUOUS_REFLECTION | FLOW_CONTINUOUS_TRANSMISSION>.has;
};
}
