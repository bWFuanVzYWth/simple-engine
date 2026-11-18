/// 积分器和光源提案共用的直接照明样本。
module "light_sampling/sample.slang";
import geometry.ray;

namespace light_sampling {
/// 一项独立 NEE 策略；pdf<=0 表示该策略没有样本，不查询 visibility。
/// direction 为单位方向；distance 用于 Beer，visibility 可使用非单位方向。
/// emission 可缓存光源值或场景私有延迟求值参数，emission_reference由场景解释。
/// 积分器只将二者透传给light_emission，不解释其编码；实际读取可延后到可见性通过后。
/// 两者不改变 pdf 或覆盖规则；昂贵求值留给通过可见性后的 light_emission。
public struct LightSample {
    public var direction : float3;
    public var emission : float3;
    public var emission_reference : uint;
    public var pdf : float;
    public var distance : float;
    public var visibility : geometry::Ray;
};
}
