# 本机配置

根目录 `local/` 已被 Git 忽略。Streamline SDK、Verus 和固定 validation 层装在 `local/tools/`，换终端或清理 Cargo 产物后仍可使用。外部仓库位置、私有环境及凭据也放在 `local/`。

在仓库根目录创建配置：

```powershell
New-Item -ItemType Directory -Force local
```

需要使用自定义工具或数据时，在 `local/` 下创建 `env.ps1`，例如设置已有 Nsight 安装的 host 目录。

```powershell
$env:NSIGHT_HOST = '<nsight-host-directory>'
```

运行依赖该设置的命令前，从仓库根目录执行 `. ./local/env.ps1`。项目工具的默认安装目录由各入口自动发现。

需要覆盖 Cargo 配置时，在 `local/` 下创建 `cargo.toml`，例如：

```toml
[build]
target-dir = "<build-cache-directory>"
```

在仓库根目录显式加载该文件：

```powershell
cargo --config local/cargo.toml check --locked
```

本机路径和凭据留在 `local/`。
