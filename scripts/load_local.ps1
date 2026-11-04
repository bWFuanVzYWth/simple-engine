# 在当前 PowerShell 会话显式加载本机配置；不回显配置值。
$localFile = Join-Path (Split-Path -Parent $PSScriptRoot) 'local/env.ps1'
if (Test-Path -LiteralPath $localFile -PathType Leaf) {
    . $localFile
    Write-Host '已加载 local/env.ps1'
} else {
    Write-Host '尚无 local/env.ps1；按 manual/local.md 创建后重新加载。'
}
