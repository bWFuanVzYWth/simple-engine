# Cornell box 参考渲染器

离屏参考渲染：wgpu 管理资源，Vulkan KHR 光追管线执行 Slang 路径追踪。

```sh
cargo run --release -p cornell_box -- --size 512 --samples 1024
```

快速检查用 `--size 128 --samples 64`。`--device` 指定 Vulkan 适配器索引；`--help` 查看参数。参考定义、来源、积分与批次等价边界见[渲染器文档](../../target/doc/cornell_box/index.html)，共享算法见[shader 文档](../../target/doc/simple_engine/shader_library/index.html)。

`--output` 指定文件前缀，默认 `target/cornell_box/reference`，适合可再生的试渲染。要保存某次实验及其测量，改用 `--output temp/cornell/实验名/reference`；同前缀会覆盖文件。

- `.exr`：线性 HDR 辐射，用于数值比较。
- `.png`：显示预览，不能用来比较 HDR 误差。
- `.json`：运行参数、构建身份、设备/驱动与主机测量，比较时与图像一起保存。

编码及裁剪语义见[输出接口](../../target/doc/cornell_box/output/fn.write.html)。达到深度护栏时提高 `--max-depth` 后重跑，不把失败视为截断估计通过。

```sh
cargo test --locked -p cornell_box
cargo test --locked -p cornell_box -- --ignored
```

第二条检查批次等价与深度护栏，需要 Vulkan 1.4 与 KHR 光追能力；初始化失败及 error 日志均导致失败。设备基线与 validation 操作见 [native 手册](../../manual/native.md)，基础模块检验见 [GPU 测试手册](../../manual/gpu-test.md)。

native 入口默认使用 mimalloc；`--no-default-features` 使用 System，wasm 不依赖 mimalloc。
