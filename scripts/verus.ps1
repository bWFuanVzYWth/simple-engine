param(
    [string]$Verus = $env:VERUS,
    [switch]$Install
)
$ErrorActionPreference = 'Stop'

$tools = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools.json') -Raw | ConvertFrom-Json
$version = $tools.verus.version
if ($Verus -and !$Install) {
    $compiler = Get-Command $Verus -CommandType Application -ErrorAction Stop
    return [pscustomobject]@{ Path = $compiler.Source; Version = $version }
}
$windowsHost = [Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT
if ([IntPtr]::Size -ne 8 -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Verus 安装器仅支持 x86_64；其它平台请自行设置 VERUS。'
}
if (!$windowsHost -and !$IsLinux) { throw '此平台请自行设置 VERUS。' }
$platform = if ($windowsHost) { 'windows-x86_64' } else { 'linux-x86_64' }
$package = $tools.verus.$platform
$taskRoot = Split-Path -Parent $PSScriptRoot
$directory = Join-Path $taskRoot "local/tools/verus/$version/$platform"
$name = if ($windowsHost) { 'verus-x86-win/verus.exe' } else { 'verus-x86-linux/verus' }
$compiler = Join-Path $directory $name
if (!$Install) {
    if (!(Test-Path -LiteralPath $compiler -PathType Leaf)) {
        throw "未找到 Verus $version。请从仓库根目录运行 ./scripts/verus.ps1 -Install，或用 -Verus 指定路径。"
    }
    return [pscustomobject]@{ Path = $compiler; Version = $version }
}
$null = New-Item -ItemType Directory -Path $directory -Force
$archive = Join-Path $directory $package.archive
if (!(Test-Path -LiteralPath $archive) -or (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.sha256) {
    $url = "https://github.com/verus-lang/verus/releases/download/release%2F$version/$($package.archive)"
    Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing -TimeoutSec 300
}
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $package.sha256) {
    throw "Verus ZIP 的 SHA256 不符：$archive"
}
if (!(Test-Path -LiteralPath $compiler)) {
    Expand-Archive -LiteralPath $archive -DestinationPath $directory -Force
}
if (!(Test-Path -LiteralPath $compiler)) { throw "缺少 Verus 编译器：$compiler" }
if (!$windowsHost) {
    & chmod +x $compiler (Join-Path (Split-Path -Parent $compiler) 'rust_verify') (Join-Path (Split-Path -Parent $compiler) 'z3')
    if ($LASTEXITCODE -ne 0) { throw 'chmod 失败' }
}
& $compiler --version
if ($LASTEXITCODE -ne 0) { throw 'Verus 初始化失败；按上述提示安装所需 Rust toolchain 后重试。' }
Write-Host "Verus 路径 $compiler"
