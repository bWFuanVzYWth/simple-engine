// 来源：Sky Tracer 9e9430d；见 crates/atmosphere/data/README.md。
module parameters;
namespace atmosphere
{
/// Earth/balanced 的固定介质、四波长及观察者参数，按源顺序排列的400 B；数组均为连续float4。
/// 距离为 km；局部 +Y 为径向上方，太阳方位 atan2(x,z)，仰角 asin(y)。
/// 数据偏移与维度须来自同一物理输入；API 不支持混合不同求解版本的 LUT。
/// mapping_version必须为63，multiple_scattering_marker必须为1，horizon_power固定为3，path_height_scale_km有限且为正。
/// 生产核不提供legacy坐标模式。
/// 散射阶由构建入口与宿主阶段调度选择，不由本参数中的迭代记录切换。
public struct Params
{
    /// 输出尺寸、积分步数与固定的多次散射标记1。
    public var output_width : uint;
    public var output_height : uint;
    public var integration_steps : uint;
    public var multiple_scattering_marker : uint;
    /// 观察者yaw、pitch、垂直FOV为弧度，海拔为km。
    public var view_yaw : float;
    public var view_pitch : float;
    public var vertical_fov : float;
    public var observer_height_km : float;
    /// 太阳方位、仰角、角半径为弧度；行星底部半径为km。
    public var sun_azimuth : float;
    public var sun_elevation : float;
    public var sun_angular_radius : float;
    public var planet_radius_km : float;
    /// source高度、太阳状态、optical高度与角度节点数。
    public var source_height_count : uint;
    public var sun_state_count : uint;
    public var optical_height_count : uint;
    public var optical_angle_count : uint;
    /// 物理输入float4表中的系数、保留位、剖面与相函数偏移。
    public var coefficient_offset : uint;
    public var reserved_coefficient_offset : uint;
    public var profile_offset : uint;
    public var phase_offset : uint;
    /// 相函数质量偏移、低空高度数、剖面高度数与保留位。
    public var phase_mass_offset : uint;
    public var low_height_count : uint;
    public var profile_height_count : uint;
    public var reserved_profile_offset : uint;
    /// 大气厚度与有限线段为km；地表反照率与固定地平线采样幂3。
    public var atmosphere_height_km : float;
    public var ground_albedo : float;
    public var segment_length_km : float;
    public var horizon_power : float;
    /// 450/510/580/650nm太阳辐照谱密度。
    public var solar_irradiance_spectrum : float4;
    /// 四波长到线性Rec.2020的拟合列，w不使用。
    public var spectral_to_rec2020 : float4[4];
    /// 入射方向、相位、cone节点数及宿主散射迭代记录。
    public var incident_direction_count : uint;
    public var phase_node_count : uint;
    public var cone_node_count : uint;
    public var scattering_iteration : uint;
    /// 太阳状态、积分角、相位节点偏移及optical积分步数。
    public var sun_state_offset : uint;
    public var integration_angle_offset : uint;
    public var phase_node_offset : uint;
    public var optical_integration_steps : uint;
    /// 固定标记63及solar、optical、cone节点偏移。
    public var mapping_version : uint;
    public var solar_node_offset : uint;
    public var optical_node_offset : uint;
    public var cone_node_offset : uint;
    /// SkyView宽高、低天空行数与全部天空行数；其余为地面chart。
    public var sky_width : uint;
    public var sky_height : uint;
    public var lower_sky_rows : uint;
    public var sky_rows : uint;
    /// 六段optical高度映射系数。
    public var optical_height_segments : float4[6];
    /// 拟合相位混合权重与cone扭曲，其余保留。
    public var phase_blend_weight : float;
    public var cone_warp : float;
    public var reserved_angular_0 : float;
    public var reserved_angular_1 : float;
    /// 太阳重要性宽度、路径高度尺度km、高太阳权重与保留位。
    public var solar_importance_width : float;
    public var path_height_scale_km : float;
    public var high_sun_weight : float;
    public var reserved_importance : float;
    /// 当前高度批首项/数量及optical行更新范围。
    public var batch_height_start : uint;
    public var batch_height_count : uint;
    public var optical_row_start : uint;
    public var optical_row_count : uint;
};

}
