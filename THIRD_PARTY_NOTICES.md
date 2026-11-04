# 第三方代码与资产

以下材料排除在本项目自有内容的 GPL-3.0-only 授权范围之外，保留原有版权、许可和声明。
对这些材料的适配不移除原作者权利。

| 材料 | 许可与声明 |
| --- | --- |
| PBRT-v4 FastOwen 采样代码，Matt Pharr、Wenzel Jakob、Greg Humphreys | [Apache-2.0](LICENSES/pbrt-Apache-2.0.txt)；版权保留于采样源码头部 |
| RoboCute OpenPBR 内核、作者修正与能量表，RoboCute contributors | [Apache-2.0](LICENSES/robocute-Apache-2.0.txt)、[来源与版权](LICENSES/robocute-NOTICE.txt) |
| NVIDIA Self Intersection Avoidance 射线数值代码 | [BSD-3-Clause 与版权](LICENSES/NVIDIA-self-intersection-avoidance.txt) |
| NVIDIA Falcor ReSTIR PT 代码与 paired LUT | [来源](shaders/restir_pt/NOTICE.md)、[BSD-3-Clause 与版权](shaders/restir_pt/LICENSE.txt)；源码保留各自声明 |
| NVIDIA Streamline 2.14.1 headers 与 DLSS 310.9.1 runtime | [Streamline 许可](LICENSES/NVIDIA-Streamline.txt)、[DLSS 许可](LICENSES/NVIDIA-DLSS.txt)、[第三方声明](LICENSES/NVIDIA-Streamline-Third-Party.md)；来源与 SHA256 见 [SDK 锁](crates/vulkan/streamline/sdk-lock.json)，SDK 缓存在 `target/streamline-sdk`，不纳入 Git |
| Khronos Vulkan-Headers，供可选 Streamline bridge 构建 | [Apache-2.0](LICENSES/Khronos-Vulkan-Headers-Apache-2.0.txt)、[MIT](LICENSES/Khronos-Vulkan-Headers-MIT.txt)；精确版本与来源见 SDK 锁 |
| Noto Sans SC 字体 | [来源](assets/fonts/README.md)、[SIL Open Font License 1.1](assets/fonts/OFL.txt) |

外部 Rust 依赖保留各自的许可证和版权；实际版本见 `Cargo.lock`，许可信息见对应上游 crate。
