# Slang 安装与构建排障

安装完整的 [Slang v2026.20 发行包](https://github.com/shader-slang/slang/releases/tag/v2026.20)。这是仓库 CI 基线，其他版本须按实际构建与测试确认兼容性。也可使用 Vulkan SDK 自带的 Slang。

按 `SLANGC` → Vulkan SDK → `PATH` 查找编译器。为减少目录扫描并消除命令查找歧义，建议指定路径：

```powershell
$env:SLANGC = "C:/tools/slang/bin/slangc.exe"
& $env:SLANGC -version
cargo build --locked -p hello_triangle
```

Windows 将依赖 DLL 保留在 `slangc.exe` 同目录；其他平台保留发行包的 bin/lib 布局。安装与动态库说明见[Slang 官方指南](https://github.com/shader-slang/slang/blob/master/docs/user-guide/01-get-started.md)。

构建脚本通过 [`shader_build::build`](../target/doc/shader_build/fn.build.html) 声明入口；recipe 示例、编译器身份、依赖跟踪和指纹字段见[构建 API](../target/doc/shader_build/index.html)。接入引擎数值模块见[共享模块操作](shaders.md)。

所有 recipe 默认 `-std 2026 -O3`。SPIR-V 在 `cargo build --profile release-lto` 使用 `-g0`，其他 profile 保持 `-g3`；根据 Cargo 包输出槽识别精确 profile，不能用自定义 profile 中仍为 `release` 的 `PROFILE` 变量判断。`-O3` 是 Slang maximal 优化，可能增加编译时间或代码尺寸。`Recipe::args` 可覆盖 SPIR-V 调试和优化级别，实际参数进入构建指纹与 `shader-build.txt`。native 直接使用 Slang 的 SPIR-V，固定启用内部验证。

WGSL 在所有 profile 固定 `-g0`，避免 Slang 2026.20 的调试保留辅助类型产生无效接口；recipe 的调试参数不能覆盖此策略。运行时编译使用相同 target/profile 默认策略，缓存身份包含实际参数；它只验证通用 SPIR-V 有效性，应用特化契约仍由实际消费者审查。

批量入口自动并行并复用内容缓存；命中／编译数、worker上限、峰值任务数和批量耗时在 `OUT_DIR/shader-build.txt`。`SHADER_BUILD_JOBS` 限制本批并行度，并受 Cargo jobserver 总令牌约束；内存紧张时降低该值。`SHADER_BUILD_CACHE` 可指定共享缓存目录，默认是 Cargo 构建目录的 `shader-cache`，可直接清理。依赖失效、生成源跨 `OUT_DIR` 复用、调试路径与审计边界见[构建契约](../target/doc/shader_build/index.html#编译器与增量构建)。

定位构建失败：

1. 用 `-version` 确认实际编译器可执行且依赖完整；检查 `SLANGC`、`VULKAN_SDK` 和 `PATH`。
2. 用 `cargo build -vv` 查看构建脚本诊断，对照错误中的入口和编译器路径。
3. 编译成功后，在 Cargo 的 `OUT_DIR/shader-build.txt` 查看实际参数与输出名；比较实验时同时保存编译器版本、构建指纹及原始报告。
4. 检查 recipe 的 source、include 路径、stage、profile 和 capability；native 使用直接 SPIR-V，Web 使用 WGSL target。布局问题同时核对受控构建的 reflection 与宿主绑定声明。

`OUT_DIR` 的产物可重新生成，不承担失败现场保存职责。需要保留的诊断、外部编译器副本或原始测量放入 `temp/`，避免清理 `target/` 时丢失。
