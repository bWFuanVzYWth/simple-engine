param(
    [string]$SdkRoot,
    [switch]$VerifyOnly,
    [string]$DeployTo
)

$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (!$SdkRoot) { $SdkRoot = $env:STREAMLINE_SDK }
if (!$SdkRoot) { $SdkRoot = Join-Path $repository 'target/streamline-sdk' }
$SdkRoot = [IO.Path]::GetFullPath($SdkRoot)
& (Join-Path $repository 'crates/vulkan/streamline/setup.ps1') -SdkRoot $SdkRoot -VerifyOnly:$VerifyOnly -DeployTo $DeployTo
$env:STREAMLINE_SDK = $SdkRoot
Write-Output "STREAMLINE_SDK=$SdkRoot"
