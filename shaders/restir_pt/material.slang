/// ReSTIR各材质操作共享的数值契约；Flow为资源期已证明的编译期控制域。
module material;
import restir_pt.scene;
import pt.scene;
import pt.transport;
import bsdf.openpbr.flow;

namespace restir_pt
{
/// 仅BSDF使用着色法线；介质跨界和路径偏移始终使用几何法线。
public func material_normal(surface: Surface, material: pt::SurfaceMaterial)->float3
{
    return select(any(material.normal != 0), material.normal, surface.point.normal);
}

/// 采样结果不携带未被point算法消费的闭包；保留精确proposal、介质和delta过滤语义。
public func material_sample<let Flow : uint>(surface: Surface, material: pt::SurfaceMaterial,
                                             sampled: openpbr::Sample)
    ->BsdfSample
{
    var result : BsdfSample = {};
    result.medium = surface.incident;
    if (sampled.events == 0)
        return result;
    var response : float3 = sampled.response;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0 &&
        (sampled.events & openpbr::DISCRETE) != 0 &&
        dot(surface.wo, surface.point.normal) * dot(sampled.direction, surface.point.normal) < 0)
        response *= material.boundary_filter;
    result.direction = sampled.direction;
    result.weight = pt::product_over(float3(1), response, sampled.pdf);
    result.pdf = sampled.pdf;
    result.componentPdf = sampled.pdf;
    result.relativeEta = 1;
    result.delta = (sampled.events & openpbr::DISCRETE) != 0;
    result.transmission = (sampled.events & openpbr::TRANSMISSION) != 0;
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0)
        result.medium = sampled.medium;
    return result;
}

/// 连接段介质；非solid程序不包含穿界分支或transmitted读取。
public func material_medium<let Flow : uint>(surface: Surface, material: pt::SurfaceMaterial,
                                             direction: float3)
    ->float4
{
    if ((Flow & openpbr::FLOW_SOLID_TRANSMISSION) != 0 &&
        dot(surface.wo, surface.point.normal) * dot(direction, surface.point.normal) < 0)
        return material.transmitted;
    return surface.incident;
}

/// 沿连接方向的实际均匀介质Beer。
public func material_transmittance<let Flow : uint>(surface: Surface, material: pt::SurfaceMaterial,
                                                    direction: float3, distance: float)
    ->float3
{
    return exp(-material_medium<Flow>(surface, material, direction).yzw * distance);
}

/// 重连第一端同时产生BSDF与Beer；第二端应使用纯evaluate_flow，不能为距离0搬运介质RGB。
public func evaluate_connection<let Flow : uint>(surface: Surface, material: pt::SurfaceMaterial,
                                                 energy: openpbr::Energy, direction: float3,
                                                 distance: float)
    ->ConnectionEvaluation
{
    let evaluated : openpbr::Evaluation = openpbr::evaluate_flow<Flow>(
                        energy, material.pbr, material_normal(surface, material), surface.wo,
                        direction, surface.incident, material.transmitted);
    let medium : float4 = material_medium<Flow>(surface, material, direction);
    return { { evaluated.response, evaluated.pdf }, exp(-medium.yzw * distance), medium.x };
}
}
