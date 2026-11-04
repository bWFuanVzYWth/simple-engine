param([string]$SevenZip)
$ErrorActionPreference = 'Stop'

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT -or
    [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw '此安装器只提供 Windows x86_64 validation；其它平台请设置 VK_LAYER_PATH。'
}
$taskRoot = Split-Path -Parent $PSScriptRoot
$tools = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools.json') -Raw | ConvertFrom-Json
$package = $tools.validation.'windows-x86_64'
$directory = Join-Path $taskRoot "target/tools/validation/$($tools.validation.version)/windows-x86_64"
$layerPath = Join-Path $directory 'bin'

function Test-Hash([string]$Path, [string]$Hash) {
    (Test-Path -LiteralPath $Path) -and
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -eq $Hash
}

$missing = @($package.files.psobject.Properties | Where-Object {
    !(Test-Hash (Join-Path $layerPath $_.Name) $_.Value)
})
if ($missing.Count -ne 0) {
    if (!$SevenZip) {
        $command = Get-Command 7z -ErrorAction SilentlyContinue
        $SevenZip = if ($command) { $command.Source } else { Join-Path $env:ProgramFiles '7-Zip/7z.exe' }
    }
    if (!(Test-Path -LiteralPath $SevenZip)) { throw '需要 7-Zip；可用 -SevenZip 指定 7z.exe。' }
    $null = New-Item -ItemType Directory -Path $layerPath -Force
    $archive = Join-Path $directory $package.archive
    if (!(Test-Hash $archive $package.sha256)) {
        $url = "https://sdk.lunarg.com/sdk/download/$($tools.validation.version)/windows/vulkan_sdk.exe"
        Invoke-WebRequest -Uri $url -OutFile $archive -UseBasicParsing
    }
    if (!(Test-Hash $archive $package.sha256)) { throw "SDK SHA256 不符：$archive" }

    # 固定发行包内的 7z 段；不执行 SDK 安装器，只提取 validation 的 DLL 与 manifest。
    $payload = Join-Path $directory 'validation.7z'
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
    & $SevenZip e -y -bso0 -bsp0 "-o$layerPath" $payload @files
    if ($LASTEXITCODE -ne 0) { throw '提取 validation 失败。' }
}
foreach ($file in $package.files.psobject.Properties) {
    if (!(Test-Hash (Join-Path $layerPath $file.Name) $file.Value)) {
        throw "validation SHA256 不符：$($file.Name)"
    }
}
$manifest = Get-Content -LiteralPath (Join-Path $layerPath 'VkLayer_khronos_validation.json') -Raw | ConvertFrom-Json
$env:VK_LAYER_PATH = $layerPath
Write-Host "validation=$($manifest.layer.api_version)"
Write-Host "VK_LAYER_PATH=$layerPath"
