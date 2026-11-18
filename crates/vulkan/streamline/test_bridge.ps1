param([string]$SdkRoot)
$ErrorActionPreference = 'Stop'
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
$SdkRoot = & (Join-Path $PSScriptRoot 'setup.ps1') -SdkRoot $SdkRoot -VerifyOnly -PassThru
$output = Join-Path $repo 'target/streamline-bridge-tests'
New-Item -ItemType Directory -Force -Path $output | Out-Null
& clang++ -std=c++17 -fexceptions `
    "-I$SdkRoot/include" "-I$SdkRoot/vulkan-headers/include" `
    "$PSScriptRoot/bridge_tests.cpp" -o "$output/bridge_tests.exe"
if ($LASTEXITCODE -ne 0) { throw 'Streamline bridge 测试编译失败。' }
& "$output/bridge_tests.exe"
if ($LASTEXITCODE -ne 0) { throw 'Streamline bridge 测试失败。' }
