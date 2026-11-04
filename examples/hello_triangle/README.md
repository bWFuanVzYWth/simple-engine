# hello_triangle

winit + wgpu 窗口三角形，Slang SPIR-V 直通；无需硬件光追。工具链见 [仓库说明](../../README.md)。

```sh
cargo run -p hello_triangle
cargo run -p hello_triangle -- --frames 4
```

零尺寸暂停，resize 重建配置，sRGB 附件编码输出。`--frames N` 按实际呈现计数，到达后退出并检查 GPU 错误。

native 入口默认使用 mimalloc；`--no-default-features` 使用 System，wasm 不依赖 mimalloc。
