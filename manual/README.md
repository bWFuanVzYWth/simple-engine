# 操作手册

第一次使用请从[快速开始](getting-started.md)进入，那里说明怎样安装工具、找到仓库目录并运行第一个示例。想把库接进自己的项目，见[引擎接入](engine.md)。本手册各页再按构建、测试和诊断分工。

## 生成文档

Rustdoc 是从源码注释生成的网页，可用浏览器阅读接口、架构和使用契约。按[快速开始](getting-started.md#打开终端并确认位置)打开 PowerShell 并进入仓库根目录。先准备 [Slang](shader-build.md)，Windows 上的全 feature（功能开关）文档还需要 [Streamline SDK](streamline.md)。

```powershell
cargo doc --locked --workspace --all-features --no-deps
```

看到 `Generated` 后，打开本机的 [引擎文档](../target/doc/simple_engine/index.html)。从左侧选对应 crate（Rust 组件）、模块或类型查看详情。需要阅读内部模块时，可在命令末尾加 `--document-private-items`。

完整文档检查与 Web 目标生成：

```powershell
powershell -NoProfile -File scripts/check.ps1 -Suite docs
```

检查公开项覆盖率、链接、HTML 与可执行示例，报告位于 `target/doc-coverage.json`；覆盖率不替代语义审查。Web API 位于 `target/wasm32-unknown-unknown/doc/`。

## 手册索引

- [快速开始](getting-started.md)
- [开发检查与维护](development.md)
- [引擎 API](../target/doc/simple_engine/index.html)：功能模块与组件入口。
- [引擎接入与功能裁剪](engine.md)
- [Slang 安装与构建排障](shader-build.md)
- [共享 Slang 模块接入](shaders.md)
- [本机路径与环境配置](local.md)
- [native 设备与 validation](native.md)
- [Streamline RR](streamline.md)
- [GPU 测试、失败重放与测量](gpu-test.md)
- [Nsight GPU Trace 离线诊断](nsight.md)
- [GPU 进程超时](gpu-runner.md)
- [Criterion CPU 基准](cpu-bench.md)

开发规则见 [AGENTS.md](../AGENTS.md)。`temp/` 存放失败现场、原始测量、计划等不可再生的本地临时材料，人工决定何时清理；`target/` 仅存可再生产物，可无条件清理。两者均不纳入版本管理；需要长期保存的材料经审查放入相应目录。
