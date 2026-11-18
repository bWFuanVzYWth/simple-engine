param(
    [string]$SdkRoot,
    [switch]$VerifyOnly,
    [string]$DeployTo,
    [switch]$PassThru
)

$ErrorActionPreference = 'Stop'
$lockFile = Join-Path $PSScriptRoot 'sdk-lock.json'
$lock = Get-Content -LiteralPath $lockFile -Raw | ConvertFrom-Json
if ($lock.schema -ne 1) { throw 'Unsupported SDK lock schema' }
if (!$SdkRoot -and $env:STREAMLINE_SDK) {
    if ($env:STREAMLINE_SDK -notmatch '^(?:[A-Za-z]:[/\\]|[/\\]{2}[^/\\])') {
        throw 'STREAMLINE_SDK 必须为绝对路径。'
    }
    $SdkRoot = $env:STREAMLINE_SDK
}
if (!$SdkRoot) {
    $lockHash = (Get-FileHash -LiteralPath $lockFile -Algorithm SHA256).Hash
    $SdkRoot = Join-Path $PSScriptRoot "../../../local/tools/streamline/$($lock.sources.streamline.tag)-$($lockHash.Substring(0, 16))"
}
$SdkRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($SdkRoot)
if ($VerifyOnly -and !(Test-Path -LiteralPath $SdkRoot -PathType Container)) {
    throw "未找到 Streamline SDK：$SdkRoot。请从仓库根目录运行 ./crates/vulkan/streamline/setup.ps1 安装锁定 SDK。"
}

function Assert-Hash([string]$Path, [string]$Expected) {
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Missing SDK file: $Path" }
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Expected) {
        throw "SDK SHA-256 mismatch: $Path"
    }
}

function Sdk-File([string]$Relative) {
    $path = [IO.Path]::GetFullPath((Join-Path $SdkRoot $Relative))
    if (!$path.StartsWith($SdkRoot.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "SDK lock path escapes SDK root: $Relative"
    }
    return $path
}

if (!$VerifyOnly) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $cache = Join-Path $SdkRoot 'downloads'
    New-Item -ItemType Directory -Path $cache -Force | Out-Null
    foreach ($property in $lock.sources.PSObject.Properties) {
        $name = $property.Name
        $files = @($lock.files | Where-Object source -eq $name)
        if (!$files.Count) { continue }
        $source = $property.Value
        $archivePath = Join-Path $cache $source.archive
        if (!(Test-Path -LiteralPath $archivePath)) {
            Invoke-WebRequest -Uri $source.url -OutFile $archivePath
        }
        Assert-Hash $archivePath $source.sha256
        $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
        try {
            foreach ($file in $files) {
                $entry = $archive.GetEntry($file.entry)
                if (!$entry) { throw "Missing archive entry: $($file.entry)" }
                $destination = Sdk-File $file.path
                New-Item -ItemType Directory -Path (Split-Path $destination -Parent) -Force | Out-Null
                [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $destination, $true)
                Assert-Hash $destination $file.sha256
            }
        } finally { $archive.Dispose() }
    }
}

foreach ($file in $lock.files) { Assert-Hash (Sdk-File $file.path) $file.sha256 }
foreach ($file in $lock.files | Where-Object path -like '*.dll') {
    $path = Sdk-File $file.path
    $signature = Get-AuthenticodeSignature -LiteralPath $path
    if ($signature.Status -ne 'Valid' -or
        $signature.SignerCertificate.Subject -notmatch '(^|, )CN=NVIDIA Corporation(,|$)') {
        throw "SDK runtime is not validly signed by NVIDIA: $path"
    }
    if ($DeployTo) {
        New-Item -ItemType Directory -Path $DeployTo -Force | Out-Null
        Copy-Item -LiteralPath $path -Destination $DeployTo -Force
    }
}
if ($DeployTo) {
    Copy-Item -LiteralPath (Sdk-File 'licenses/NVIDIA-Streamline.txt'),(Sdk-File '3rd-party-licenses.md'),(Sdk-File 'bin/x64/nvngx_dlss.license.txt') -Destination $DeployTo -Force
}
if ($PassThru) {
    Write-Output $SdkRoot
} else {
    Write-Output "已校验 $($lock.files.Count) 个 SDK 文件（Streamline $($lock.sources.streamline.tag)、DLSS $($lock.sources.dlss.tag)）"
    Write-Output "SDK 目录 $SdkRoot"
}
