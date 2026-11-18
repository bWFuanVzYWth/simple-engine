// SPDX-FileCopyrightText: Copyright (c) 2026 linlin
// SPDX-License-Identifier: GPL-3.0-only AND Apache-2.0
/// 已绑定recipe选择实际物理部件；数值实现按底层、测度、反射和混合分别维护。
module composition;
import openpbr.selection;
__exported import openpbr.closure.contracts;
__exported import openpbr.closure.base;
__exported import openpbr.closure.measure;
__exported import openpbr.closure.reflection;
__exported import openpbr.closure.mixture;

namespace openpbr {
namespace closure {
static const let base_is_mixed
    : bool = (selected_flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE)) == (FLOW_DIFFUSE | FLOW_SUBSURFACE);
static const let base_is_binary : bool = (selected_variant & variant_subsurface_switch) != 0;
static const let base_has_diffuse : bool = (selected_flow & FLOW_DIFFUSE) != 0;
static const let base_has_subsurface : bool = (selected_flow & FLOW_SUBSURFACE) != 0;
typealias WeightedBase = TypeSelect<base_is_binary, SubsurfaceSwitch, SubsurfaceBlend>.Type;
typealias DiffuseBase = TypeSelect<base_has_diffuse, Lambert, Absorber>.Type;
typealias UnmixedBase = TypeSelect<base_has_subsurface, Subsurface, DiffuseBase>.Type;
public typealias SelectedBase = TypeSelect<base_is_mixed, WeightedBase, UnmixedBase>.Type;

static const let reflection_is_mixed
    : bool = (selected_flow & (FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION)) ==
             (FLOW_DELTA_REFLECTION | FLOW_CONTINUOUS_REFLECTION);
static const let reflection_is_continuous : bool =
                                                (selected_flow & FLOW_CONTINUOUS_REFLECTION) != 0;
typealias SingleMeasure = TypeSelect<reflection_is_continuous, Continuous, Smooth>.Type;
public typealias SelectedMeasure =
    TypeSelect<reflection_is_mixed, MixedMeasure, SingleMeasure>.Type;

static const let opaque_has_reflection : bool = (selected_flow & FLOW_DIELECTRIC_REFLECTION) != 0;
static const let opaque_has_base : bool = (selected_flow & (FLOW_DIFFUSE | FLOW_SUBSURFACE)) != 0;
static const let opaque_has_dielectric
    : bool = (selected_flow & (FLOW_DIFFUSE | FLOW_DIELECTRIC_REFLECTION | FLOW_SUBSURFACE)) != 0;
static const let opaque_has_metal : bool = (selected_flow & FLOW_CONDUCTOR_REFLECTION) != 0;
static const let opaque_is_mixture : bool = (selected_flow & FLOW_METAL_MIX) != 0;
static const let opaque_shares_reflection
    : bool = (selected_flow & (FLOW_DIELECTRIC_REFLECTION | FLOW_CONTINUOUS_REFLECTION)) ==
             (FLOW_DIELECTRIC_REFLECTION | FLOW_CONTINUOUS_REFLECTION);
typealias SpecularOnly =
    TypeSelect<reflection_is_continuous, Specular<SelectedMeasure>, DeltaDielectric>.Type;
typealias ReflectingDielectric =
    TypeSelect<opaque_has_base, DielectricLayer<SelectedMeasure, SelectedBase>, SpecularOnly>.Type;
public typealias Dielectric =
    TypeSelect<opaque_has_reflection, ReflectingDielectric, BaseClosure<SelectedBase>>.Type;
public typealias Metal = Conductor<SelectedMeasure>;
typealias MixedOpaque = MetalMixture<opaque_shares_reflection, SelectedMeasure, Dielectric>.Type;
typealias WeightedOpaque = TypeSelect<opaque_has_dielectric, MixedOpaque, NullMetal<Metal>>.Type;
typealias UnmixedOpaque = TypeSelect<opaque_has_metal, Metal, Dielectric>.Type;
public typealias Opaque = TypeSelect<opaque_is_mixture, WeightedOpaque, UnmixedOpaque>.Type;
}
}
