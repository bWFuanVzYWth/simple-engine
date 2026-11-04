# wgpu / Vulkan 集成测试

验证同设备 wgpu / ash 的交接与资源生命周期。需要 Slang 和 Vulkan；RT 用例的 Vulkan 1.4 与扩展要求见 [native 手册](../../manual/native.md)，普通 compute 不要求光追，窗口用例另需桌面环境。全部用例默认 ignored：

```powershell
cargo test -p wgpu_vulkan_tests --test interop -- --ignored --test-threads=1
cargo test -p wgpu_vulkan_tests --test instances -- --ignored --test-threads=1
cargo test -p wgpu_vulkan_tests --test sampled -- --ignored --test-threads=1
cargo test -p wgpu_vulkan_tests --test callable -- --ignored --test-threads=1
cargo test -p wgpu_vulkan_tests --test buffers -- --ignored --test-threads=1
cargo test -p wgpu_vulkan_tests --test native -- --ignored --test-threads=1
```

| 关注点 | 检查的不变量 |
|---|---|
| 三阶段交接 | wgpu compute → ash RT → wgpu compute 的逐像素数据正确 |
| 跨帧复用 | 同一组共享纹理与管线连续使用时结果正确 |
| 共享 buffer 初始化 | GPU 清零后局部写入生效，其余字节保持零；提交后可释放调用方引用 |
| 采样资源交接 | wgpu 写 D3 HALF4／D2 FLOAT4 → native 线性采样／Load → wgpu 改写，拒绝异设备 owner |
| 叶子 callable | 动态 SBT 索引、连续 payload 往返、buffer／采样图像读取，以及调用方释放后的 SBT／管线保活 |
| 通用 native | 不要求 position fetch／OMM 的 any-hit→compute、同队列提交保活及 sRGB 上传回读 |
| 在途资源保活 | 替换资源并释放旧引用后，已提交的工作仍能完成 |
| KHR 2-state OMM | 透明/不透明、重复 map 与 fully opaque special 在释放调用方引用后仍正确 |
| 并发提交 | 多线程提交并释放引用后，各批工作仍能完成 |
| 窗口 resize | resize 并替换共享资源后，仍可继续呈现且无验证错误 |

组件由 `simple_engine` 入口提供。前四项通过 `gpu_test::with_shared` 复用一个 GPU 环境；窗口项由独立 `window_probe` 进程运行，满足 winit 的主线程要求。该进程是测试夹具，不是应用入口。

仅运行离屏用例时，在命令末尾追加 `"[rt]"`。

Debug 默认启用 validation。带外层超时和同步验证的完整执行入口为 `scripts/check.ps1 -Suite rt`；初始化失败、device lost、validation Error 和析构错误使运行失败，未执行不算通过。

日志使用引擎 `gpu_log`，不维护测试专用 logger；全进程累计检查约定见 [引擎入口](../../target/doc/gpu_log/index.html)。

native shader 直接使用 Slang SPIR-V；互操作契约见 [native 互操作](../../manual/native.md)。这些用例不承担性能测量，也不覆盖 SER 或 Streamline。
