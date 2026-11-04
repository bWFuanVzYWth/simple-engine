<#
.SYNOPSIS
验证 SipHash 同源实现与入口契约，并检查违规反例。
.DESCRIPTION
固定 Verus 版本，先编译最小标准库可信适配，再以 --no-cheating 执行完整正例及逐项反例。
可信适配编译不计为证明通过；负例只接受指定断言或 unwind 保证失败，初始化、编译错误、超时均失败。
原始日志与状态保存于 temp/verus-sip-hash。
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
$run = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$logs = Join-Path $taskRoot "temp/verus-sip-hash/$run"
$build = Join-Path $taskRoot "target/verus-sip-hash/$run"
$null = [IO.Directory]::CreateDirectory($logs)
$null = [IO.Directory]::CreateDirectory($build)
$clock = [Diagnostics.Stopwatch]::StartNew()
$trustedSource = 'crates/sip_hash/verify/std.rs'
$summary = [ordered]@{ Version = $tools.verus.version; Status = 'failed'; TrustedSource = $trustedSource; TrustedCompiled = $false; Verified = 0; Rejected = @(); Seconds = 0.0 }
$cases = @('reject_seed_mapping', 'reject_coordinate_order', 'reject_missing_tail', 'reject_lane_reorder', 'reject_zero_round_output', 'reject_unwind_contract')

try {
    $compiler = (Get-Command $Verus -ErrorAction Stop).Source
    $version = Invoke-CheckedProcess -FilePath $compiler -ArgumentList @('--version') -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'version')
    $versionText = [IO.File]::ReadAllText($version.StdoutPath) + [IO.File]::ReadAllText($version.StderrPath)
    if ($versionText -notmatch ('Version:\s*' + [regex]::Escape($tools.verus.version) + '(\s|$)')) {
        throw "Verus 版本与 scripts/tools.json 不符：$($tools.verus.version)"
    }
    $trustedModel = Join-Path $build 'sip_hash_std.vir'
    $trustedLibrary = Join-Path $build 'libsip_hash_std.rlib'
    $trustedArguments = @('--edition=2024', '--crate-name', 'sip_hash_std', '--crate-type=lib', '--rlimit', '10', '--export', $trustedModel, '--compile', '-o', $trustedLibrary, $trustedSource)
    $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList $trustedArguments -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'trusted-std')
    if (!(Test-Path -LiteralPath $trustedModel) -or !(Test-Path -LiteralPath $trustedLibrary)) {
        throw '可信适配未生成完整 Rust/Verus 库'
    }
    $summary.TrustedCompiled = $true
    $arguments = @('--edition=2024', '--no-cheating', '--rlimit', '10', '--triggers-mode', 'silent', '--extern', "sip_hash_std=$trustedLibrary", '--import', "sip_hash_std=$trustedModel", 'crates/sip_hash/verify/hash.rs')
    $positive = Invoke-CheckedProcess -FilePath $compiler -ArgumentList $arguments -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'hash')
    $result = [IO.File]::ReadAllText($positive.StdoutPath) + [IO.File]::ReadAllText($positive.StderrPath)
    if ($result -notmatch 'verification results::\s*([1-9][0-9]*) verified, 0 errors') {
        throw '未得到非空且无错误的 SipHash 验证结果'
    }
    $summary.Verified = [int]$Matches[1]
    foreach ($case in $cases) {
        $prefix = Join-Path $logs $case
        $rejected = $false
        try {
            $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList ($arguments + @('--cfg', $case, '--verify-root', '--verify-function', $case)) -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix $prefix
        } catch {
            if ($_.Exception.Data['ExitCode'] -ne 1 -or $_.Exception.Data['TimedOut']) { throw }
            $diagnostic = [IO.File]::ReadAllText("$prefix.stdout.log") + [IO.File]::ReadAllText("$prefix.stderr.log")
            if ($diagnostic -notmatch 'verification results::\s*[0-9]+ verified, [1-9][0-9]* errors') { throw }
            if ($case -eq 'reject_unwind_contract') {
                if ($diagnostic -notmatch 'cannot show this call will not unwind, in function marked `no_unwind`' -or $diagnostic -notmatch 'call to (?:hash::)?unspecified_unwind might unwind') { throw }
            } elseif ($diagnostic -notmatch 'assertion failed') { throw }
            $rejected = $true
        }
        if (!$rejected) { throw "负例未被 Verus 拒绝：$case" }
        $summary.Rejected += $case
        Write-Host "已拒绝：$case"
    }
    $summary.Status = 'passed'
    Write-Host "SipHash 同源实现与 $($cases.Count) 个负例通过；日志 $logs"
} catch {
    $summary.Error = $_.Exception.Message
    [Console]::Error.WriteLine($_.Exception.Message)
    [Console]::Error.WriteLine("日志：$logs")
    $code = $_.Exception.Data['ExitCode']
    if ($null -eq $code) { exit 1 }
    exit [int]$code
} finally {
    $clock.Stop()
    $summary.Seconds = $clock.Elapsed.TotalSeconds
    $summary | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $logs 'summary.json') -Encoding UTF8
}
