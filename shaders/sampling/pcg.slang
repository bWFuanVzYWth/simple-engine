/// PCG RXS-M-XS 的 hash 和状态更新；由调用方分配种子和随机流。
/// 来源：https://www.jcgt.org/published/0009/03/02/
module pcg;

namespace sampling {
/// 对一个 32 位值执行 PCG RXS-M-XS hash，不保存或推进调用方状态。
public func pcg(value: uint)->uint {
    let state = value * 747796405u + 2891336453u;
    let word : uint = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

/// 推进一次 32 位状态并返回 [0, 1)；使用输出高 24 位，避免 float32 舍入到 1。
/// 调用方须保持所需的种子／流分离和维度消费顺序，本函数不为像素或路径分配随机流。
public func random(inout state: uint)->float {
    state = state * 747796405u + 2891336453u;
    let word : uint = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return float(((word >> 22u) ^ word) >> 8u) * 5.9604644775390625e-8;
}
}
