/// 初采交接按读写阶段复用槽位。命中程序从内建量取得几何；已知表面的 callable 另带几何身份。
module initial_call;
import restir_pt.scene;

namespace restir_pt
{
/// 输出为唯一胜出的直接光候选、命中发光和累计权重，共25个word。
public struct InitialPacket
{
    public var direct : InitialDirect;
    public var emission : float3;
    public var emission_pdf : float;
    public var weight : float;

    public static func input_hit(medium: float4, request: InitialRequest)->InitialPacket
    {
        var p : InitialPacket = {};
        p.direct.direction = request.previous_position;
        p.direct.radiance = request.throughput;
        p.direct.reflected = medium.xyz;
        p.direct.pdf = request.weight;
        p.direct.bsdf_pdf = request.previous_mis_pdf;
        p.direct.geometry_factor = medium.w;
        p.direct.light_type = request.bounce_seed;
        p.weight = float(request.flags);
        return p;
    }

    public static func input(s: Surface, request: InitialRequest, attributes: uint4)->InitialPacket
    {
        var p : InitialPacket = input_hit(s.incident, request);
        p.direct.attenuated_radiance = s.point.position;
        p.direct.hit.data = attributes;
        p.emission = s.wo;
        p.emission_pdf = s.point.offset;
        p.weight = float(request.flags | (uint(s.front_facing) << 6u));
        return p;
    }

    public func request()->InitialRequest
    {
        return { direct.light_type, uint(weight) & 63u, direct.direction,
                 direct.radiance,   direct.pdf,         direct.bsdf_pdf };
    }

    public func surface(normal: float3, token: uint2)->Surface
    {
        var s : Surface = {};
        s.incident = float4(direct.reflected, direct.geometry_factor);
        s.point = { direct.attenuated_radiance, normal, emission_pdf };
        s.wo = emission;
        s.identity = direct.hit.data;
        s.hit.data = uint4(token, s.identity.zw);
        s.front_facing = (uint(weight) & 64u) != 0;
        return s;
    }

    [mutating]
    public func store(v: InitialVertex)->void
    {
        direct = v.direct;
        emission = v.emission;
        emission_pdf = v.emission_pdf;
        weight = v.weight;
    }

    public func result()->InitialVertex
    {
        var v : InitialVertex = {};
        v.direct = direct;
        v.emission = emission;
        v.emission_pdf = emission_pdf;
        v.weight = weight;
        return v;
    }
};

public struct InitialSampleCall
{
    public var vertex : InitialPacket;
    public var direction : float3;
    public var response : float3;
    public var medium : float4;
    public var pdf : float;
    public var flags : uint;

    public static func input(s: Surface, r: InitialRequest, attributes: uint4)->InitialSampleCall
    {
        var c : InitialSampleCall = {};
        c.vertex = InitialPacket::input(s, r, attributes);
        c.direction = s.point.normal;
        c.response.xy = asfloat(s.hit.data.xy);
        return c;
    }

    public func surface()->Surface
    {
        return vertex.surface(direction, asuint(response.xy));
    }
    public func request()->InitialRequest
    {
        return vertex.request();
    }

    [mutating]
    public func store(v: InitialVertex)->void
    {
        vertex.store(v);
        direction = v.sample.direction;
        response = v.sample.weight;
        medium = v.sample.medium;
        pdf = v.sample.pdf;
        flags = uint(v.sample.delta) | (uint(v.sample.transmission) << 1u);
    }

    public func result()->InitialVertex
    {
        var v : InitialVertex = vertex.result();
        v.sample.direction = direction;
        v.sample.weight = response;
        v.sample.medium = medium;
        v.sample.pdf = pdf;
        v.sample.component_pdf = pdf;
        v.sample.relative_eta = 1;
        v.sample.delta = (flags & 1u) != 0;
        v.sample.transmission = (flags & 2u) != 0;
        return v;
    }
};

/// 真命中终止入口只需权重、随机与介质，几何由内建量提供。
public struct InitialHitDirectCall
{
    public var vertex : InitialPacket;
    public func surface()->Surface
    {
        return vertex.surface(0, 0);
    }
    public func request()->InitialRequest
    {
        return vertex.request();
    }
    [mutating]
    public func store(v: InitialVertex)->void
    {
        vertex.store(v);
    }
    public func result()->InitialVertex
    {
        return vertex.result();
    }
};

/// 已知表面没有硬件命中内建量，五个附加输入仅在调用前有效。
public struct InitialKnownDirectCall
{
    public var vertex : InitialPacket;
    public var normal : float3;
    public var token : uint2;

    public static func input(s: Surface, r: InitialRequest, attributes: uint4)
        ->InitialKnownDirectCall
    {
        return { InitialPacket::input(s, r, attributes), s.point.normal, s.hit.data.xy };
    }
    public func surface()->Surface
    {
        return vertex.surface(normal, token);
    }
    public func request()->InitialRequest
    {
        return vertex.request();
    }
    [mutating]
    public func store(v: InitialVertex)->void
    {
        vertex.store(v);
    }
    public func result()->InitialVertex
    {
        return vertex.result();
    }
};
}
