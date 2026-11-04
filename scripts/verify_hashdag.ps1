<#
.SYNOPSIS
验证 HashDag 首批五项性质的生产同源入口与违规反例。
.DESCRIPTION
固定 Verus 与 Rust 依赖编译器；hashbrown 与标准原语为可信边界，身份发行核单独验证。
八个算法入口不使用局部过滤；身份发行核完整检查本模块及子模块。
任何缺失、编译错误、超时或证明失败均使入口失败。
#>
param(
    [string]$Verus = $env:VERUS,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 180
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
. (Join-Path $PSScriptRoot 'process.ps1')
$taskRoot = Split-Path -Parent $PSScriptRoot
$tools = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'tools.json') -Raw | ConvertFrom-Json
if (!$Verus) {
    $binary = if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) { 'verus-x86-win/verus.exe' } else { 'verus-x86-linux/verus' }
    $Verus = Join-Path $taskRoot "target/tools/verus/$($tools.verus.version)/$binary"
}
$run = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$logs = Join-Path $taskRoot "temp/verus-hashdag/$run"
$build = Join-Path $taskRoot "target/verus-hashdag/$run"
$deps = Join-Path $taskRoot 'target/verus-hashdag-deps'
$null = [IO.Directory]::CreateDirectory($logs)
$null = [IO.Directory]::CreateDirectory($build)
$clock = [Diagnostics.Stopwatch]::StartNew()
$summary = [ordered]@{ Version = $tools.verus.version; Status = 'failed'; TrustedCompiled = $false; OwnedIdentity = $null; Entries = @(); Rejected = @(); Seconds = 0.0 }
$entries = @('node', 'roots', 'storage', 'canonical', 'write', 'diff', 'rc', 'owner')
$cases = @(
    @{ Source = 'node'; Cfg = 'reject_node_mask'; Function = 'reject_node_mask' },
    @{ Source = 'node'; Cfg = 'reject_node_fold'; Function = 'reject_node_fold' },
    @{ Source = 'node'; Cfg = 'reject_pointer_dedup'; Function = 'reject_pointer_dedup' },
    @{ Source = 'node'; Cfg = 'reject_morton_axis'; Function = 'reject_morton_axis' },
    @{ Source = 'roots'; Cfg = 'reject_stale_root'; Function = 'identity_roundtrip' },
    @{ Source = 'roots'; Cfg = 'reject_foreign_root'; Function = 'foreign_rejected' },
    @{ Source = 'roots'; Cfg = 'reject_root_generation_wrap'; Function = 'saturation_preserves' },
    @{ Source = 'storage'; Cfg = 'reject_storage_merge'; Function = 'reject_storage_merge' },
    @{ Source = 'storage'; Cfg = 'reject_storage_existing_edges'; Function = 'reject_storage_existing_edges' },
    @{ Source = 'storage'; Cfg = 'reject_storage_rollback'; Function = 'reject_storage_rollback' },
    @{ Source = 'canonical'; Cfg = 'reject_semantic_identity'; Function = 'reject_semantic_identity' },
    @{ Source = 'write'; Cfg = 'reject_cell_first_writer'; Function = 'reject_cell_first_writer' },
    @{ Source = 'diff'; Cfg = 'reject_diff_old_reuse'; Function = 'reject_diff_old_reuse' },
    @{ Source = 'diff'; Cfg = 'reject_diff_missing_new'; Function = 'reject_diff_missing_new' },
    @{ Source = 'diff'; Cfg = 'reject_diff_error_output'; Function = 'reject_diff_error_output' },
    @{ Source = 'rc'; Cfg = 'reject_rc_no_decrement'; Function = 'reject_rc_no_decrement' },
    @{ Source = 'rc'; Cfg = 'reject_rc_edge_set'; Function = 'reject_rc_edge_set' },
    @{ Source = 'rc'; Cfg = 'reject_rc_shared_drop'; Function = 'reject_rc_shared_drop' },
    @{ Source = 'owner'; Cfg = 'reject_owner_fork_alias'; Function = 'reject_owner_fork_alias' },
    @{ Source = 'owner'; Cfg = 'reject_owner_shared_release'; Function = 'reject_owner_shared_release' },
    @{ Source = 'owner'; Cfg = 'reject_owner_stale_read'; Function = 'reject_owner_stale_read' },
    @{ Source = 'owner'; Cfg = 'reject_owner_number_reuse'; Function = 'distinct_owners' }
)
try {
    $identityPrimitives = @('primitive_new', 'permissioned_try_update', 'global_counter')
    foreach ($source in @('crates/hashdag/src/identity.rs', 'crates/hashdag/src/proof/identity_owner.rs')) {
        $owned = [IO.File]::ReadAllText((Join-Path $taskRoot $source))
        if ($owned -match '\b(assume|admit|uninterp|assume_specification|external(?:_\w+)?)\b' -or $owned -match '\binclude(?:_str|_bytes)?\s*!') {
            throw "身份发行核出现未授权的可信声明：$source"
        }
        $paths = @([regex]::Matches($owned, '#\[path\s*=\s*"([^"]+)"\]') | ForEach-Object { $_.Groups[1].Value })
        $modules = @([regex]::Matches($owned, '\bmod\s+((?:r#)?\w+)\s*([;{])') | ForEach-Object { $_.Groups[1].Value + $_.Groups[2].Value })
        if ($source -eq 'crates/hashdag/src/identity.rs') {
            if ($paths.Count -ne 1 -or $paths[0] -ne 'proof/identity_owner.rs' -or (Compare-Object @('proof;', 'tests{') $modules)) { throw '身份发行核的依赖闭包改变，须先审查并更新检查入口' }
        } elseif ($paths.Count -ne 0 -or $modules.Count -ne 0) { throw '身份辅助证明不得绕过入口加载额外模块' }
    }
    $identityOps = [IO.File]::ReadAllText((Join-Path $taskRoot 'crates/hashdag/src/identity_ops.rs'))
    $primitiveNames = @([regex]::Matches($identityOps, '(?s)#\[cfg_attr\(verus_only,\s*verifier::external_body\)\].*?\bfn\s+([A-Za-z_]\w*)') | ForEach-Object { $_.Groups[1].Value })
    if ($identityOps -match '\b(assume|admit|uninterp|assume_specification)\b' -or $identityOps -match '\binclude(?:_str|_bytes)?\s*!' -or [regex]::Matches($identityOps, '\bexternal(?:_\w+)?\b').Count -ne 3 -or [regex]::Matches($identityOps, 'verifier::external_body').Count -ne 3 -or $primitiveNames.Count -ne 3 -or (Compare-Object $identityPrimitives $primitiveNames)) {
        throw '身份适配只允许初始化、线性化 RMW 与同一 static 对象关联三个可信函数'
    }
    $opsPaths = @([regex]::Matches($identityOps, '#\[path\s*=\s*"([^"]+)"\]') | ForEach-Object { $_.Groups[1].Value })
    $opsModules = @([regex]::Matches($identityOps, '\bmod\s+((?:r#)?\w+)\s*([;{])') | ForEach-Object { $_.Groups[1].Value + $_.Groups[2].Value })
    if ($opsPaths.Count -ne 1 -or $opsPaths[0] -ne '../verify/identity.rs' -or (Compare-Object @('model;') $opsModules)) { throw '身份标准适配的依赖闭包改变' }
    $identityModel = [IO.File]::ReadAllText((Join-Path $taskRoot 'crates/hashdag/verify/identity.rs'))
    $modelNames = @([regex]::Matches($identityModel, '\buninterp\s+spec\s+fn\s+(\w+)') | ForEach-Object { $_.Groups[1].Value })
    if ($identityModel -match '\b(assume|admit|assume_specification)\b' -or $identityModel -match '\binclude(?:_str|_bytes)?\s*!|\bmod\s|#\[path\b' -or [regex]::Matches($identityModel, '\bexternal(?:_\w+)?\b').Count -ne 1 -or [regex]::Matches($identityModel, 'verifier::external_body').Count -ne 1 -or [regex]::Matches($identityModel, '\buninterp\b').Count -ne 3 -or $identityModel -notmatch 'pub tracked struct Permission' -or $modelNames.Count -ne 3 -or (Compare-Object @('view', 'cell', 'global_authority') $modelNames)) {
        throw '身份可信模型只允许不透明 Permission 及许可、单元、静态 authority 三个元数据视图'
    }
    $compiler = (Get-Command $Verus -ErrorAction Stop).Source
    $version = Invoke-CheckedProcess -FilePath $compiler -ArgumentList @('--version') -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'version')
    $versionText = [IO.File]::ReadAllText($version.StdoutPath) + [IO.File]::ReadAllText($version.StderrPath)
    if ($versionText -notmatch ('Version:\s*' + [regex]::Escape($tools.verus.version) + '(\s|$)')) {
        throw "Verus 版本与 scripts/tools.json 不符：$($tools.verus.version)"
    }
    $cargo = (Get-Command cargo -ErrorAction Stop).Source
    $dependencyStdout = [Console]::Out
    try {
        [Console]::SetOut([IO.TextWriter]::Null)
        $dependencies = Invoke-CheckedProcess -FilePath $cargo -ArgumentList @('+1.99.1', 'build', '--manifest-path', 'crates/hashdag/verify/Cargo.toml', '--locked', '--target-dir', $deps, '--message-format=json') -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'dependencies')
    } finally { [Console]::SetOut($dependencyStdout) }
    $artifacts = @{}
    foreach ($line in [IO.File]::ReadLines($dependencies.StdoutPath)) {
        if (!$line.StartsWith('{')) { continue }
        $message = $line | ConvertFrom-Json
        if ($message.reason -ne 'compiler-artifact') { continue }
        foreach ($file in $message.filenames) {
            if ($file.EndsWith('.rlib')) { $artifacts[$message.target.name] = $file }
        }
    }
    $external = @('-L', ('dependency=' + (Join-Path $deps 'debug/deps')))
    foreach ($dependency in @('hashbrown', 'allocator_api2', 'bytemuck')) {
        if (!$artifacts.ContainsKey($dependency)) { throw "依赖产物缺失：$dependency" }
        $external += @('--extern', ($dependency + '=' + $artifacts[$dependency]))
    }
    $trustedModel = Join-Path $build 'hash_dag_std.vir'
    $trustedLibrary = Join-Path $build 'libhash_dag_std.rlib'
    $trustedArgs = @('--edition=2024', '--crate-name', 'hash_dag_std', '--crate-type=lib', '--rlimit', '10', '--triggers-mode', 'silent', '--export', $trustedModel, '--compile', '-o', $trustedLibrary) + $external + @('crates/hashdag/verify/std.rs')
    $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList $trustedArgs -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'trusted-std')
    if (!(Test-Path -LiteralPath $trustedModel) -or !(Test-Path -LiteralPath $trustedLibrary)) { throw '可信适配产物不完整' }
    $summary.TrustedCompiled = $true
    # --no-cheating 按整 crate 扫描，不能排除同一导出库中的三个标准原语。
    # 发行核不允许可信声明，并单独检查 identity 及其全部子模块；不记为 TCB。
    $identityArgs = @('--edition=2024', '--crate-name', 'hash_dag_std', '--crate-type=lib', '--rlimit', '10', '--triggers-mode', 'silent', '--verify-module', 'identity') + $external + @('crates/hashdag/verify/std.rs')
    $identity = Invoke-CheckedProcess -FilePath $compiler -ArgumentList $identityArgs -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'owned-identity')
    $identityText = [IO.File]::ReadAllText($identity.StdoutPath) + [IO.File]::ReadAllText($identity.StderrPath)
    $identityVerified = [regex]::Match($identityText, 'verification results::\s*([1-9][0-9]*) verified, 0 errors')
    if (!$identityVerified.Success) { throw '身份发行同源核未通过' }
    $summary.OwnedIdentity = [ordered]@{ Name = 'identity'; Verified = [int]$identityVerified.Groups[1].Value; TrustedFunctions = $identityPrimitives; TrustedModel = 'crates/hashdag/verify/identity.rs' }
    $arguments = @('--edition=2024', '--no-cheating', '--rlimit', '10', '--triggers-mode', 'silent', '--extern', "hash_dag_std=$trustedLibrary", '--import', "hash_dag_std=$trustedModel") + $external
    foreach ($entry in $entries) {
        $source = "crates/hashdag/verify/$entry.rs"
        if (!(Test-Path -LiteralPath (Join-Path $taskRoot $source))) { throw "维护入口缺失：$source" }
        $positive = Invoke-CheckedProcess -FilePath $compiler -ArgumentList ($arguments + @($source)) -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs $entry)
        $result = [IO.File]::ReadAllText($positive.StdoutPath) + [IO.File]::ReadAllText($positive.StderrPath)
        $verified = [regex]::Match($result, 'verification results::\s*([1-9][0-9]*) verified, 0 errors')
        if (!$verified.Success -or $result -match 'partial verification') { throw "完整入口未通过：$entry" }
        $summary.Entries += [ordered]@{ Name = $entry; Verified = [int]$verified.Groups[1].Value }
    }
    foreach ($case in $cases) {
        $prefix = Join-Path $logs $case.Cfg
        $rejected = $false
        try {
            $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList ($arguments + @("crates/hashdag/verify/$($case.Source).rs", '--cfg', $case.Cfg, '--verify-root', '--verify-function', $case.Function)) -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix $prefix
        } catch {
            if ($_.Exception.Data['ExitCode'] -ne 1 -or $_.Exception.Data['TimedOut']) { throw }
            $diagnostic = [IO.File]::ReadAllText("$prefix.stdout.log") + [IO.File]::ReadAllText("$prefix.stderr.log")
            if ($diagnostic -match 'resource limit|rlimit.*exceeded|timed out|error\[E[0-9]+\]|internal (compiler )?error|thread .*panicked') { throw }
            if ($diagnostic -notmatch 'verification results::\s*[0-9]+ verified, [1-9][0-9]* errors' -or $diagnostic -notmatch 'assertion failed') { throw }
            $rejected = $true
        }
        if (!$rejected) { throw "负例未被拒绝：$($case.Cfg)" }
        $summary.Rejected += $case.Cfg
    }
    $summary.Status = 'passed'
    Write-Host "HashDag 全部维护入口与 $($cases.Count) 个反例通过；日志 $logs"
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
    $summary | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $logs 'summary.json') -Encoding UTF8
}
