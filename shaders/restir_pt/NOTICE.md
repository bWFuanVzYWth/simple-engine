# Falcor ReSTIR PT 默认 paired LUT

来源：[Falcor](https://github.com/NVIDIAGameWorks/Falcor/tree/759aad03)，Git `759aad03`，NVIDIA BSD-3-Clause，完整条款见本目录 `LICENSE.txt`。算法源码各自保留原版权、条款和免责声明。

输入：`Source/Modules/ReSTIRPathTracing/PairedReusePattern/neighborCount3-stdev16.0/neighbor{0,1,2}.png`。Falcor默认K=3、spatialPairingStdevLevel=5，平均半径30，经`stddev=sqrt(8/(9*pi))*30`格式化一位小数得到16.0。本目录保留原LUT数值，不生成替代邻居。

`paired.bin`为三个slot，每slot65536个little-endian u32（总786432B）。每个PNG按原row-major读取16-bit整数，提升为u32后在slot尾部补0；真实边长见下表，不能按256×256解释有效像素。低8bits为dx+128，高8bits为dy+128。GPU绑定StructuredBuffer<uint>，不需要PNG运行时依赖。

| slot | 边长 | 原PNG SHA-256 |
|---|---:|---|
| 0 | 254 | `daa416e7258eb18c0a5b819abf18091095d286df430cd5719d237335094dc027` |
| 1 | 232 | `b8b8d6a268ef9a2df73f4f92e856b2adb294dc0eef843a0b5f8c4046eab3d861` |
| 2 | 196 | `20a7c5c17aa7464bd5c8653d57732c29a9c6d559b145a97df51d0edf68fee471` |

paired.bin SHA-256：`1ad261567560e966404848c2d0fd3798c4eb76e6fef0c44a5af2ef3d9e169cb9`。

转换时已逐有效像素验证每slot在其周期域满足delta(p+delta(p))=-delta(p)。shader保留NV整帧共享平移、反射、转置的原顺序；屏幕边缘拒绝越界邻居，不做画面wrap。
