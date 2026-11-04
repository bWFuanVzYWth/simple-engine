/// 无资源绑定的 float32 采样内核；种子、随机流和样本维度顺序由调用方决定。
/// PCG RXS-M-XS 来源：https://www.jcgt.org/published/0009/03/02/
module sampling;

namespace pt
{
/// 圆周率，按 float32 舍入。
public static const float pi = 3.14159265358979323846;

/// 对一个 32 位值执行 PCG RXS-M-XS hash，不保存或推进调用方状态。
public uint pcg(uint value)
{
    uint state = value * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

/// 推进一次 32 位状态并返回 [0, 1)；使用输出高 24 位，避免 float32 舍入到 1。
/// 调用方须保持所需的种子／流分离和维度消费顺序，本函数不为像素或路径分配随机流。
public float random(inout uint state)
{
    state = state * 747796405u + 2891336453u;
    uint word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return float(((word >> 22u) ^ word) >> 8u) * 5.9604644775390625e-8;
}

/// normal 须为单位向量，u 属于 [0, 1)²；返回同坐标系内 normal 正半球的单位方向。
/// PDF 为 cos(theta)/pi，采用立体角量度 sr^-1，不包含随机数生成。
/// 极坐标圆盘映射：https://pbr-book.org/4ed/Sampling_Algorithms/Sampling_Multidimensional_Functions
public float3 cosine_hemisphere(float3 normal, float2 u)
{
    float phi = 2.0 * pi * u.y;
    float3 tangent =
        normalize(cross(abs(normal.z) < 0.999 ? float3(0, 0, 1) : float3(0, 1, 0), normal));
    float3 bitangent = cross(normal, tangent);
    return sqrt(u.x) * (cos(phi) * tangent + sin(phi) * bitangent) + sqrt(1.0 - u.x) * normal;
}
}
