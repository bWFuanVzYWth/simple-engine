param()
$ErrorActionPreference = 'Stop'

$taskRoot = Split-Path -Parent $PSScriptRoot
$tools = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools.json') -Raw | ConvertFrom-Json
$windowsHost = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
if ([IntPtr]::Size -ne 8 -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Slang 发行包仅支持 x86_64；其它平台请自行设置 SLANGC。'
}
$platform = if ($windowsHost) { 'windows-x86_64' } else { 'linux-x86_64' }
if (!$windowsHost -and !$IsLinux) { throw '此平台请自行设置 SLANGC。' }
$package = $tools.slang.$platform
$directory = Join-Path $taskRoot "target/tools/slang/$($tools.slang.version)/$platform"
$null = New-Item -ItemType Directory -Path $directory -Force
$archive = Join-Path $directory $package.archive
if (!(Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.sha256) {
    $url = "https://github.com/shader-slang/slang/releases/download/v$($tools.slang.version)/$($package.archive)"
    Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.sha256) {
    throw "Slang ZIP 的 SHA256 不符：$archive"
}
Expand-Archive -LiteralPath $archive -DestinationPath $directory -Force
$name = if ($windowsHost) { 'bin/slangc.exe' } else { 'bin/slangc' }
$compiler = Join-Path $directory $name
if (!(Test-Path -LiteralPath $compiler)) { throw "缺少 Slang 编译器：$compiler" }
if (!$windowsHost) {
    & chmod +x $compiler
    if ($LASTEXITCODE -ne 0) { throw 'chmod 失败' }
}
$env:SLANGC = $compiler
& $compiler -version
if ($LASTEXITCODE -ne 0) { throw 'Slang 初始化失败' }
Write-Host "SLANGC=$compiler"
