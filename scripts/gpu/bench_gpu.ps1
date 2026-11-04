param(
    [Parameter(Mandatory = $true)][string]$Before,
    [Parameter(Mandatory = $true)][string]$After,
    [string]$Output = (Join-Path 'temp/gpu-ab' ([DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8))),
    [ValidateRange(1, 1000)][int]$Pairs = 7,
    [ValidateRange(1, 16777216)][int]$Count = 65536,
    [ValidateRange(1, 65536)][int]$Dispatches = 64,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 300
)
$ErrorActionPreference = "Stop"
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
. (Join-Path $PSScriptRoot "../process.ps1")
$executables = @{
    before = (Resolve-Path -LiteralPath $Before).Path
    after = (Resolve-Path -LiteralPath $After).Path
}
$directory = (New-Item -ItemType Directory -Path $Output -Force).FullName
$previousDebug = $env:WGPU_DEBUG
$previousValidation = $env:WGPU_VALIDATION
$previousSync = $env:VK_LAYER_VALIDATE_SYNC
$rows = @()
$runs = @()

function Median($values) {
    $sorted = @($values | Sort-Object)
    $middle = [int][Math]::Floor($sorted.Count / 2)
    if ($sorted.Count % 2) { return $sorted[$middle] }
    return ($sorted[$middle - 1] + $sorted[$middle]) / 2
}

try {
    $env:WGPU_DEBUG = "0"
    $env:WGPU_VALIDATION = "0"
    $env:VK_LAYER_VALIDATE_SYNC = $null
    for ($pair = 0; $pair -lt $Pairs; $pair++) {
        $order = if ($pair % 2) { @("after", "before") } else { @("before", "after") }
        $reports = @{}
        foreach ($variant in $order) {
            $path = Join-Path $directory "$pair-$variant.json"
            $null = Invoke-CheckedProcess -FilePath $executables[$variant] -ArgumentList @(
                "--count", "$Count", "--dispatches", "$Dispatches", "--rounds", "1", "--output", $path
            ) -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $directory "$pair-$variant")
            $reports[$variant] = Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
            $runs += [pscustomobject]@{ pair = $pair; variant = $variant; report = $path }
        }
        foreach ($field in @("device", "driver", "slang", "seed", "scale")) {
            $a = $reports.before.$field | ConvertTo-Json -Compress
            $b = $reports.after.$field | ConvertTo-Json -Compress
            if ($a -ne $b) { throw "A/B 的 $field 不同，不能直接配对" }
        }
        $beforeLoads = @($reports.before.workloads)
        $afterLoads = @($reports.after.workloads)
        if ($beforeLoads.Count -ne $afterLoads.Count) { throw "A/B 负载集合不同" }
        foreach ($load in $beforeLoads) {
            $matches = @($afterLoads | Where-Object name -EQ $load.name)
            if ($matches.Count -ne 1) { throw "A/B 负载不匹配：$($load.name)" }
            $other = $matches[0]
            foreach ($field in @("input_blake3", "work_items", "count", "dispatches")) {
                if ($load.$field -ne $other.$field) { throw "$($load.name) 的 $field 不同" }
            }
            $a = $load.samples[0]
            $b = $other.samples[0]
            $ratio = if ($a.gpu_seconds -gt 0) { $b.gpu_seconds / $a.gpu_seconds } else { $null }
            $rows += [pscustomobject]@{
                pair = $pair; workload = $load.name
                before_gpu_seconds = $a.gpu_seconds; after_gpu_seconds = $b.gpu_seconds
                gpu_ratio = $ratio
                before_host_seconds = $a.host_seconds; after_host_seconds = $b.host_seconds
            }
        }
    }
} catch {
    if ($null -ne $_.Exception.Data['ExitCode']) {
        Write-Error $_ -ErrorAction Continue
        exit ([int]$_.Exception.Data['ExitCode'])
    }
    throw
} finally {
    $env:WGPU_DEBUG = $previousDebug
    $env:WGPU_VALIDATION = $previousValidation
    $env:VK_LAYER_VALIDATE_SYNC = $previousSync
}

$summaries = @($rows | Group-Object workload | ForEach-Object {
    $ratios = @($_.Group.gpu_ratio | Where-Object { $null -ne $_ })
    [pscustomobject]@{
        workload = $_.Name; pairs = $_.Count
        median_gpu_ratio = if ($ratios.Count) { Median $ratios } else { $null }
        min_gpu_ratio = ($ratios | Measure-Object -Minimum).Minimum
        max_gpu_ratio = ($ratios | Measure-Object -Maximum).Maximum
    }
})
$rows | Export-Csv -LiteralPath (Join-Path $directory "pairs.csv") -NoTypeInformation -Encoding UTF8
[pscustomobject]@{
    format = "gpu-ab-v1"; before = $executables.before; after = $executables.after
    timing = "GPU timestamp；host 包含录制、提交、等待和查询回读；每次进程独立热身"
    runs = $runs; summaries = $summaries
} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $directory "summary.json") -Encoding UTF8
$summaries | Format-Table -AutoSize
