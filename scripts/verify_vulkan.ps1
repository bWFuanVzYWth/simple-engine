<#
.SYNOPSIS
验证 Vulkan 同源资源状态核和协议模型，并检查各项违规反例。
.DESCRIPTION
同源入口装配生产模块，另检查独立协议模型；不编译或运行 Vulkan 后端。负例只接受证明失败，
工具初始化、Rust 编译错误和超时均视为检查失败。日志保存于 temp/verus。
#>
param(
    [string]$Verus = $env:VERUS,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 120
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
. (Join-Path $PSScriptRoot 'process.ps1')
$taskRoot = Split-Path -Parent $PSScriptRoot
$tools = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools.json') -Raw | ConvertFrom-Json
if (!$Verus) {
    $name = if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { 'verus-x86-win/verus.exe' } else { 'verus-x86-linux/verus' }
    $Verus = Join-Path $taskRoot "target/tools/verus/$($tools.verus.version)/$name"
}
$compiler = (Get-Command $Verus -ErrorAction Stop).Source
$run = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$logs = Join-Path $taskRoot "temp/verus/$run"
$clock = [Diagnostics.Stopwatch]::StartNew()
$targets = @(
    @{ Name = 'arena'; Cases = @('reject_stale_arena_key', 'reject_cursor_skips_live_value') },
    @{ Name = 'resource'; Cases = @('reject_resource_missing_edge', 'reject_resource_rank_reversal', 'reject_producer_changes_dependency', 'reject_raw_changes_resource_link', 'reject_producer_changes_payload') },
    @{ Name = 'graph'; Cases = @('reject_graph_missing_dependency', 'reject_graph_unreachable_keep', 'reject_graph_unmarked_leaf', 'reject_graph_marked_generation') },
    @{ Name = 'lifecycle'; Cases = @('reject_resource_double_take', 'reject_resource_generation_revival', 'reject_lifecycle_ready_mutation', 'reject_lifecycle_retiring_publication', 'reject_lifecycle_query_link_change', 'reject_lifecycle_groups_unique', 'reject_batch_duplicate_acceptance', 'reject_batch_failed_prefix', 'reject_batch_quarantine_acceptance', 'reject_record_double_take', 'reject_record_generation_revival', 'reject_record_external_cancel', 'reject_record_cancelled_producer', 'reject_record_attempt_reversion', 'reject_record_completed_producer', 'reject_published_root_order', 'reject_record_root_reset', 'reject_collection_live_consumer', 'reject_collection_partial_commit') },
    @{ Name = 'retirement'; Cases = @('reject_retirement_wait_growth', 'reject_retirement_republish', 'reject_retirement_duplicate_assignment', 'reject_retirement_live_wait_release') },
    @{ Name = 'submission'; Cases = @('reject_unaccepted_completion', 'reject_failed_device_reuse', 'reject_submission_serial_wrap') },
    @{ Name = 'idle'; Cases = @('reject_idle_reorder', 'reject_idle_overflow') },
    @{ Name = 'recovery'; Cases = @('reject_reset_loss', 'reject_unconfigured_recording') },
    @{ Name = 'lifetime'; Cases = @('reject_early_release', 'reject_stale_key', 'reject_pending_release', 'reject_external_release') }
)
$summary = @()

try {
    $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList @('--version') -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'version')
    foreach ($target in $targets) {
        $model = "crates/vulkan/verify/$($target.Name).rs"
        $arguments = @('--edition=2024', '--no-cheating', '--rlimit', '10', $model)
        $positive = Invoke-CheckedProcess -FilePath $compiler -ArgumentList $arguments -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs $target.Name)
        $result = [IO.File]::ReadAllText($positive.StdoutPath) + [IO.File]::ReadAllText($positive.StderrPath)
        if ($result -notmatch 'verification results::\s*([1-9][0-9]*) verified, 0 errors') {
            throw "未得到非空且无错误的 Verus 验证结果：$($target.Name)"
        }
        $summary += [pscustomobject]@{ Target = $target.Name; Verified = [int]$Matches[1]; Seconds = $positive.ElapsedSeconds; Rejected = $target.Cases.Count }
        foreach ($case in $target.Cases) {
            $prefix = Join-Path $logs $case
            $rejected = $false
            try {
                $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList ($arguments + @('--cfg', $case)) -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix $prefix
            } catch {
                if ($_.Exception.Data['ExitCode'] -ne 1 -or $_.Exception.Data['TimedOut']) { throw }
                $diagnostic = [IO.File]::ReadAllText("$prefix.stdout.log") + [IO.File]::ReadAllText("$prefix.stderr.log")
                if ($diagnostic -notmatch 'verification results::\s*[0-9]+ verified, [1-9][0-9]* errors' -or $diagnostic -notmatch 'precondition not satisfied|assertion failed') { throw }
                $rejected = $true
            }
            if (!$rejected) { throw "负例未被 Verus 拒绝：$case" }
            Write-Host "已拒绝：$case"
        }
    }
    $clock.Stop()
    $summary | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $logs 'summary.json') -Encoding UTF8
    $summary | Format-Table -AutoSize
    Write-Host ('Verus 同源核、协议模型和负例通过，耗时 {0:F2}s；日志 {1}' -f $clock.Elapsed.TotalSeconds, $logs)
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    [Console]::Error.WriteLine("日志：$logs")
    $code = $_.Exception.Data['ExitCode']
    if ($null -eq $code) { exit 1 }
    exit [int]$code
}
