# CIE 可见光数据

International Commission on Illumination（CIE），维也纳，2019。两张原表和随包分发的合并数据均采用 [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/)。

| 数据 | 原始 CSV 的 SHA-256 |
| --- | --- |
| [CIE 1931 标准观察者，2°](https://doi.org/10.25039/CIE.DS.xvudnb9b) | `fa663e3535a7e0763a745993a1f0a192eb0275ac46ad2d1befd7626841e713c1` |
| [CIE 标准照明体 D65](https://doi.org/10.25039/CIE.DS.hjfjmt59) | `e76f210bffff3d552ef7113025da5f325d5dfec200dd4b878b1a2f3a507032cb` |

`cie1931_d65.f64le` 由两张 CSV 按波长连接，保留从 360 到 830 nm 的共同区间，共 471 行。每行依次写入 x̄、ȳ、z̄ 和 D65 相对功率四个 IEEE 754 小端 f64。原表中的每个整 nm 都保留，波长由行号给出。数据定义和插值约定见[观察者元数据](https://files.cie.co.at/Publications-datasets/CIE_xyz_1931_2deg.csv_metadata.json)和[D65 元数据](https://files.cie.co.at/Publications-datasets/CIE_std_illum_D65.csv_metadata_v2.json)。
