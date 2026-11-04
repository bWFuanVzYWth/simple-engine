param(
    [Parameter(Mandatory = $true)][string]$SdkRoot,
    [switch]$VerifyOnly,
    [string]$DeployTo
)

$ErrorActionPreference = 'Stop'
$SdkRoot = [IO.Path]::GetFullPath($SdkRoot)
$lock = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'sdk-lock.json') -Raw | ConvertFrom-Json
if ($lock.schema -ne 1) { throw 'Unsupported SDK lock schema' }

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
Write-Output "Verified $($lock.files.Count) SDK files; Streamline 2.14.1 / DLSS 310.9.1."
