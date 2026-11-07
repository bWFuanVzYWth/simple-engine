/// 初采交接按读写阶段复用槽位。命中程序从内建量取得几何；已知表面的 callable 另带几何身份。
module initial_call;
import restir_pt.scene;

namespace restir_pt
{
/// 输出为唯一胜出的直接光候选、命中发光和累计权重，共25个word。
public struct InitialPacket
{
    public InitialDirect direct;
    public float3 emission;
    public float emissionPdf;
    public float weight;

    public static InitialPacket inputHit(float4 medium, InitialRequest request)
    {
        InitialPacket p = {};
        p.direct.direction = request.previousPosition;
        p.direct.radiance = request.throughput;
        p.direct.reflected = medium.xyz;
        p.direct.pdf = request.weight;
        p.direct.bsdfPdf = request.previousMisPdf;
        p.direct.geometryFactor = medium.w;
        p.direct.lightType = request.bounceSeed;
        p.weight = float(request.flags);
        return p;
    }

    public static InitialPacket input(Surface s, InitialRequest request, uint4 attributes)
    {
        InitialPacket p = inputHit(s.incident, request);
        p.direct.attenuatedRadiance = s.point.position;
        p.direct.hit.data = attributes;
        p.emission = s.wo;
        p.emissionPdf = s.point.offset;
        p.weight = float(request.flags | (uint(s.frontFacing) << 6u));
        return p;
    }

    public InitialRequest request()
    {
        return { direct.lightType, uint(weight) & 63u, direct.direction,
                 direct.radiance,  direct.pdf,         direct.bsdfPdf };
    }

    public Surface surface(float3 normal, uint2 token)
    {
        Surface s = {};
        s.incident = float4(direct.reflected, direct.geometryFactor);
        s.point = { direct.attenuatedRadiance, normal, emissionPdf };
        s.wo = emission;
        s.identity = direct.hit.data;
        s.hit.data = uint4(token, s.identity.zw);
        s.frontFacing = (uint(weight) & 64u) != 0;
        return s;
    }

    [mutating]
    public void store(InitialVertex v)
    {
        direct = v.direct;
        emission = v.emission;
        emissionPdf = v.emissionPdf;
        weight = v.weight;
    }

    public InitialVertex result()
    {
        InitialVertex v = {};
        v.direct = direct;
        v.emission = emission;
        v.emissionPdf = emissionPdf;
        v.weight = weight;
        return v;
    }
};

public struct InitialSampleCall
{
    public InitialPacket vertex;
    public float3 direction;
    public float3 response;
    public float4 medium;
    public float pdf;
    public uint flags;

    public static InitialSampleCall input(Surface s, InitialRequest r, uint4 attributes)
    {
        InitialSampleCall c = {};
        c.vertex = InitialPacket::input(s, r, attributes);
        c.direction = s.point.normal;
        c.response.xy = asfloat(s.hit.data.xy);
        return c;
    }

    public Surface surface()
    {
        return vertex.surface(direction, asuint(response.xy));
    }
    public InitialRequest request()
    {
        return vertex.request();
    }

    [mutating]
    public void store(InitialVertex v)
    {
        vertex.store(v);
        direction = v.sample.direction;
        response = v.sample.weight;
        medium = v.sample.medium;
        pdf = v.sample.pdf;
        flags = uint(v.sample.delta) | (uint(v.sample.transmission) << 1u);
    }

    public InitialVertex result()
    {
        InitialVertex v = vertex.result();
        v.sample.direction = direction;
        v.sample.weight = response;
        v.sample.medium = medium;
        v.sample.pdf = pdf;
        v.sample.componentPdf = pdf;
        v.sample.relativeEta = 1;
        v.sample.delta = (flags & 1u) != 0;
        v.sample.transmission = (flags & 2u) != 0;
        return v;
    }
};

/// 真命中终止入口只需权重、随机与介质，几何由内建量提供。
public struct InitialHitDirectCall
{
    public InitialPacket vertex;
    public Surface surface()
    {
        return vertex.surface(0, 0);
    }
    public InitialRequest request()
    {
        return vertex.request();
    }
    [mutating]
    public void store(InitialVertex v)
    {
        vertex.store(v);
    }
    public InitialVertex result()
    {
        return vertex.result();
    }
};

/// 已知表面没有硬件命中内建量，五个附加输入仅在调用前有效。
public struct InitialKnownDirectCall
{
    public InitialPacket vertex;
    public float3 normal;
    public uint2 token;

    public static InitialKnownDirectCall input(Surface s, InitialRequest r, uint4 attributes)
    {
        return { InitialPacket::input(s, r, attributes), s.point.normal, s.hit.data.xy };
    }
    public Surface surface()
    {
        return vertex.surface(normal, token);
    }
    public InitialRequest request()
    {
        return vertex.request();
    }
    [mutating]
    public void store(InitialVertex v)
    {
        vertex.store(v);
    }
    public InitialVertex result()
    {
        return vertex.result();
    }
};
}
