# 操作手册

本目录记录构建、运行、诊断与维护操作。接口、设计和使用契约随代码生成 rustdoc，不在手册维护第二份说明。

从仓库根目录生成本地 API 文档（需要 Slang）：

```powershell
cargo doc --locked --workspace --all-features --no-deps
```

完整文档检查与 Web 目标生成：

```powershell
powershell -NoProfile -File scripts/check.ps1 -Suite docs
```

检查公开项覆盖率、链接、HTML 与可执行示例，报告位于 `target/doc-coverage.json`；覆盖率不替代语义审查。Web API 位于 `target/wasm32-unknown-unknown/doc/`。

- [引擎 API](../target/doc/simple_engine/index.html)：功能模块与组件入口。
- [引擎接入与功能裁剪](engine.md)
- [Slang 安装与构建排障](shader-build.md)
- [共享 Slang 模块接入](shaders.md)
- [本机路径与环境配置](local.md)
- [native 设备与 validation](native.md)
- [GPU 测试、失败重放与测量](gpu-test.md)
- [Nsight GPU Trace 离线诊断](nsight.md)
- [GPU 进程超时](gpu-runner.md)
- [Criterion CPU 基准](cpu-bench.md)

开发规则见 [AGENTS.md](../AGENTS.md)。`temp/` 存放失败现场、原始测量、计划等不可再生的本地临时材料，人工决定何时清理；`target/` 仅存可再生产物，可无条件清理。两者均不纳入版本管理；需要长期保存的材料经审查放入相应目录。
