$ErrorActionPreference = 'Stop'

$taskRoot = (& git -C $PSScriptRoot rev-parse --show-toplevel)
if ($LASTEXITCODE -ne 0)
{ throw '无法定位 Git 仓库。'
}

& git -C $taskRoot config --local core.hooksPath .githooks
if ($LASTEXITCODE -ne 0)
{ throw 'Git 钩子安装失败。'
}

Write-Host '已启用 .githooks，提交前检查暂存的 Rust 和 Slang 格式。'
