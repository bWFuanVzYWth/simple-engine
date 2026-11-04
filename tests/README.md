# 集成测试

跨组件契约的回归放在此目录；应用用法放在 `examples`。

- [wgpu / Vulkan](wgpu_vulkan/README.md)：交接数据、资源生命周期、并发提交与窗口 resize。
- [Shader](shaders/README.md)（包名 `shader_tests`）：直接检验生产 Slang 模块的性质、失败重放与测量；协议见 [GPU 测试](../manual/gpu-test.md)。
- [2D 渲染](render2d/README.md)：生产 Slang 的混合、裁剪、文字、多视图与在途资源生命周期。
- [OpenPBR](openpbr/README.md)：真实 HALF4 能量纹理与介质／BSDF 性质；大气原 WGSL 对照位于 `crates/atmosphere`。

测试通过 `simple_engine` 选择组件；GPU 验证日志使用引擎 `gpu_log`，见 [入口与功能裁剪](../manual/engine.md)。

GPU 用例默认 ignored；按设备能力显式运行，初始化失败和未执行均不算通过。统一检查入口见 [开发脚本](../manual/development.md)。
