# 本机配置

根目录 `local/` 已被 Git 忽略，用于本机工具路径、外部仓库位置、私有环境及凭据。共享的 `.cargo/config.toml` 只保留仓库相对设置与公共构建基线。

在仓库根目录创建配置：

```powershell
New-Item -ItemType Directory -Force local
Copy-Item scripts/local.example.ps1 local/env.ps1
```

编辑 `local/env.ps1`，只启用需要的项目；然后在每个 PowerShell 会话中显式加载：

```powershell
. ./scripts/load_local.ps1
```

加载器不回显配置值。不要覆盖已有本机配置；脚本、IDE 与子进程只继承其启动会话的环境。通用工具安装器仍可直接设置当前会话环境。

Cargo 本机覆盖使用单独文件：

```powershell
Copy-Item scripts/local.cargo.example.toml local/cargo.toml
cargo --config local/cargo.toml check --locked
```

Cargo 不自动读取此文件；共享配置不依赖 `local/`。不要把真实本机值复制到受版本管理的模板、测试 fixture 或文档。公开的作者署名、上游 URL、版本与测试所需的通用路径样例继续保留。
