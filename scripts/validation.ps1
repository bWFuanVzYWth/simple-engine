param([switch]$Install)
$ErrorActionPreference = 'Stop'

if (!$Install -and $env:VK_LAYER_PATH) { return $env:VK_LAYER_PATH }
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    if ($Install) { throw '固定 validation 安装仅支持 Windows x86_64。' }
    return
}
$taskRoot = Split-Path -Parent $PSScriptRoot
$tools = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools.json') -Raw | ConvertFrom-Json
$package = $tools.validation.'windows-x86_64'
$directory = Join-Path $taskRoot "local/tools/validation/$($tools.validation.version)/windows-x86_64"
$layerPath = Join-Path $directory 'bin'

function Test-Hash([string]$Path, [string]$Hash) {
    (Test-Path -LiteralPath $Path) -and
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $Hash
}

$missing = @($package.files.psobject.Properties | Where-Object {
    !(Test-Hash (Join-Path $layerPath $_.Name) $_.Value)
})
if (!$Install -and !(Test-Path -LiteralPath $layerPath)) { return }
if ($Install -and $missing.Count -ne 0) {
    $tar = Join-Path ([Environment]::SystemDirectory) 'tar.exe'
    if (!(Test-Path -LiteralPath $tar -PathType Leaf)) { throw '需要系统 tar.exe 解包，或使用已安装的 Vulkan SDK。' }
    $cache = Join-Path $taskRoot "target/tools/validation/$($tools.validation.version)"
    $null = New-Item -ItemType Directory -Path $cache -Force
    $null = New-Item -ItemType Directory -Path $directory -Force
    $archive = Join-Path $cache $package.archive
    if (!(Test-Hash $archive $package.sha256)) {
        $url = "https://sdk.lunarg.com/sdk/download/$($tools.validation.version)/windows/vulkan_sdk.exe"
        Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing
    }
    if (!(Test-Hash $archive $package.sha256)) { throw "SDK SHA256 不符：$archive" }

    # 固定发行包内的 7z 段；不执行 SDK 安装器，只提取 validation 的 DLL 与 manifest。
    $payload = Join-Path $cache 'validation.7z'
    if (!(Test-Hash $payload $package.payload_sha256)) {
        $inputStream = [IO.File]::OpenRead($archive)
        try {
            $null = $inputStream.Seek($package.payload_offset, [IO.SeekOrigin]::Begin)
            $outputStream = [IO.File]::Create($payload)
            try {
                $remaining = [long]$package.payload_bytes
                $buffer = New-Object byte[] 65536
                while ($remaining -gt 0) {
                    $count = $inputStream.Read($buffer, 0, [int][Math]::Min($remaining, $buffer.Length))
                    if ($count -eq 0) { throw 'SDK 内嵌 7z 段不完整。' }
                    $outputStream.Write($buffer, 0, $count)
                    $remaining -= $count
                }
            } finally { $outputStream.Dispose() }
        } finally { $inputStream.Dispose() }
    }
    if (!(Test-Hash $payload $package.payload_sha256)) { throw "内嵌 7z SHA256 不符：$payload" }
    $files = @($package.files.psobject.Properties | ForEach-Object { "Bin/$($_.Name)" })
    & $tar -xf $payload -C $directory @files
    if ($LASTEXITCODE -ne 0) { throw '提取 validation 失败。' }
}
foreach ($file in $package.files.psobject.Properties) {
    if (!(Test-Hash (Join-Path $layerPath $file.Name) $file.Value)) {
        throw "validation SHA256 不符：$($file.Name)"
    }
}
if ($Install) {
    $manifest = Get-Content -LiteralPath (Join-Path $layerPath 'VkLayer_khronos_validation.json') -Raw | ConvertFrom-Json
    Write-Host "Validation $($manifest.layer.api_version)"
    Write-Host "层目录 $layerPath"
} else {
    Write-Output $layerPath
}
