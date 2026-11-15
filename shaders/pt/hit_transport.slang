/// 主射线路由：几何查表、SER与完整材质closest-hit；无材质散射/灯采样分支。
module hit_transport;
import pt.material_vertex;
import pt.scene;
import pt.integrator;

namespace pt
{
/// program只读取将实际几何映射到已发布材质程序所需的紧凑元数据，不解码表面。
/// 辅助输出只在首表面且显式开启时调用，不成为普通路径的共同解码前缀。
public interface IHitScene : IScene
{
    func acceleration()->RaytracingAccelerationStructure;
    func program(object: HitObject, incident: float4)->uint;
    func guides_enabled()->bool;
};

/// 只调度下一段主射线，材质closest-hit独占每次命中的解码和完整路径状态更新。
/// miss保留公共环境处理；辅助量观察独立于照明路径。Reorder 在编译期选择 SER。
public func trace_hit_path<Scene : IHitScene, Guides : IHitGuides, let Reorder : bool>(
    provider: Scene, guides: Guides, inout data: TransportCall)
    ->float
{
    var distance : float = FAR;
    while (data.path.status == PATH_ACTIVE)
    {
        let ray : RayDesc = { data.path.origin, 0, data.path.direction, FAR };
        var unused : uint = 0;
        // 不将HitObject嵌入聚合状态（Slang 2026.20驱动互操作约束）。
        var found : HitObject = HitObject::TraceRay(provider.acceleration(), RAY_FLAG_NONE, 0xff, 0,
                                                    1, 0, ray, unused);
        if (!found.IsHit())
        {
            if (data.path.depth == 0)
                guides.environment(data);
            escape(provider, data.path);
            break;
        }
        if (data.path.depth == 0)
        {
            distance = found.GetRayDesc().TMax;
            if (provider.guides_enabled())
            {
                guides.surface_hit(found, data);
            }
        }
        let program : uint = provider.program(found, data.path.medium);
        found.SetShaderTableIndex(program + 1u);
        var call : MaterialCall = MaterialCall::input(data);
        // SBT记录已选择最终材质入口；让驱动按真实命中程序重排，不再把槽号当通用hint。
        if (Reorder)
            ReorderThread(found);
        HitObject::Invoke(found, call);
        data.path = call.result();
    }
    return distance;
}
}
