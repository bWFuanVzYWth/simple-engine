param([string]$SdkRoot = $env:STREAMLINE_SDK)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $SdkRoot) { throw '请设置 STREAMLINE_SDK 或传入 -SdkRoot，指向 scripts/install_streamline.ps1 安装的 SDK。' }
$SdkRoot = (Resolve-Path -LiteralPath $SdkRoot).Path
& "$repo/crates/vulkan/streamline/setup.ps1" -SdkRoot $SdkRoot -VerifyOnly
if (-not $?) { throw 'Streamline SDK 校验失败。' }
$output = Join-Path $repo 'target/streamline-bridge-tests'
New-Item -ItemType Directory -Force -Path $output | Out-Null
& clang++ -std=c++17 -fexceptions `
    "-I$SdkRoot/include" "-I$SdkRoot/vulkan-headers/include" `
    "$repo/crates/vulkan/streamline/bridge_tests.cpp" -o "$output/bridge_tests.exe"
if ($LASTEXITCODE -ne 0) { throw 'Streamline bridge 测试编译失败。' }
& "$output/bridge_tests.exe"
if ($LASTEXITCODE -ne 0) { throw 'Streamline bridge 测试失败。' }
