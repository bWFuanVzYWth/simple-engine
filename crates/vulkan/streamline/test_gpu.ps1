param(
    [string]$SdkRoot,
    [switch]$Manual,
    [switch]$InitializationOnly,
    [switch]$ReportOnlyValidation,
    [switch]$StandardValidationOnly,
    [string]$Output
)

$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../../..'))
if (!$Output) { $Output = Join-Path $root ('temp/streamline-test/' + (Get-Date -Format 'yyyyMMdd-HHmmss-fff')) }
$Output = [IO.Path]::GetFullPath($Output)
if ((Test-Path -LiteralPath $Output) -and (Get-ChildItem -LiteralPath $Output -Force)) {
    throw 'Use an empty evidence directory.'
}
New-Item -ItemType Directory -Path $Output -Force | Out-Null
$SdkRoot = & (Join-Path $PSScriptRoot 'setup.ps1') -SdkRoot $SdkRoot -VerifyOnly -DeployTo $Output -PassThru
$executable = Join-Path $Output 'rr_gpu_test.exe'
& clang++ '-std=c++17' '-O2' '-DNDEBUG' '-Wno-ignored-pragmas' `
    '-I' (Join-Path $SdkRoot 'include') `
    '-I' (Join-Path $SdkRoot 'vulkan-headers/include') `
    (Join-Path $PSScriptRoot 'gpu_test.cpp') '-o' $executable
if ($LASTEXITCODE -ne 0) { throw 'Streamline GPU fixture compilation failed' }
$arguments = @('--vulkan13', '--omit-specular-distance')
if (!$Manual) { $arguments += '--interposed' }
if ($InitializationOnly) { $arguments += '--initialization-only' }
if ($ReportOnlyValidation) { $arguments += '--report-only-validation' }
if ($StandardValidationOnly) { $arguments += '--standard-validation-only' }
$metadata = [ordered]@{
    started_utc = [DateTime]::UtcNow.ToString('o')
    interposed = !$Manual
    initialization_only = [bool]$InitializationOnly
    validation = $true
    synchronization_validation = !$StandardValidationOnly
    abort_on_validation_error = !$ReportOnlyValidation
    arguments = $arguments
    bridge_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'bridge.cpp')).Hash
    fixture_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'gpu_test.cpp')).Hash
    executable_sha256 = (Get-FileHash -LiteralPath $executable).Hash
    sdk_lock_sha256 = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'sdk-lock.json')).Hash
}
$previousLayerPath = $env:VK_LAYER_PATH
try {
    $env:VK_LAYER_PATH = & (Join-Path $root 'scripts/validation.ps1')
    & $executable @arguments 2>&1 | Tee-Object -FilePath (Join-Path $Output 'output.log')
    $exitCode = $LASTEXITCODE
} finally {
    $env:VK_LAYER_PATH = $previousLayerPath
}
$metadata.exit_code = $exitCode
$metadata.finished_utc = [DateTime]::UtcNow.ToString('o')
$metadata | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $Output 'run.json') -Encoding utf8
if ($exitCode -ne 0) { throw "Streamline GPU fixture failed ($exitCode): $Output/output.log" }
Write-Output "Streamline GPU fixture passed: $Output"
