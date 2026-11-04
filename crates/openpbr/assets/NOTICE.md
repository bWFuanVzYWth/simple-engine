# OpenPBR 支持子域来源

支持子域包括 opaque／metallic、solid／thin dielectric、
薄层 SSS 的 eval、sample、完整边缘 PDF 与能量补偿。不是完整 OpenPBR 标准实现；
排除非零 coat／fuzz／thin-film／dispersion／衍射、厚体 SSS、应用预设与自动 foliage。

数学内核的上游为 [RoboCute](https://github.com/RoboCute/RoboCute)
`0d982c77b3fd26c2c5a3c0852be3bd05e5860bd8`，含原作者 2026-07-24 dielectric-highlight hotfix。
`robocute.lock.json` 记录上游版本、作者修正及六个原始文件的 SHA-256；
`trans_ggx.bytes` 原样保存 44×32×159 HALF4，SHA-256：
`605c9160fb9348a1d033321c40cf9930226ce74c03f2624033f5b73aacfa67df`。
它是方向能量系数，不是 RGB 图像；normalized linear clamp 过滤与原版相同。

`reference/` 保留原作者头文件和说明，作为来源／oracle 参考，不是生产构建输入。
原作者 Apache-2.0 许可及声明见 `robocute-Apache-2.0.txt`、`robocute-NOTICE.txt`。

本次仅重定位 module/import 路径，并增加无应用依赖的 API、有效 CPU 材料类型、
独立 texture/sampler 能量 provider。薄片厚度由调用方显式提供；不携带 MERS 字节量化、材质 ID、纹理 preset 或介质初值策略。
