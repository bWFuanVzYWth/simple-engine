<#
.SYNOPSIS
检查矩形分解公共入口的同源调用链、几何最优性及违规反例。
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
$logs = Join-Path $taskRoot "temp/verus/rectangles/$run"
$adapter = Join-Path $taskRoot "target/verus/rectangles/$run"
$metadata = Join-Path $adapter 'rectangle_std.vir'
$library = Join-Path $adapter 'librectangle_std.rlib'
$targets = @(
    @{ Name = 'std_checks'; Kind = '标准库绑定检查（含可信规格）'; Cases = @('reject_std_count_width', 'reject_std_empty_count', 'reject_std_fill_value', 'reject_std_flatten_value', 'reject_std_simd_lane', 'reject_std_simd_high') },
    @{ Name = 'implementation'; Kind = '正式分解调用链与最优性'; Source = 'crates/rectangle_decomposition/src/lib.rs'; Args = @('--crate-type', 'lib', '-Zcrate-attr=feature(proc_macro_hygiene)'); Cases = @() },
    @{ Name = 'axis'; Kind = '同源输入、切线及输出链'; Cases = @(
        'reject_axis_row_bit', 'reject_axis_zero_boundary', 'reject_axis_overlap_boundary', 'reject_axis_row_label', 'reject_axis_column_overlap',
        'reject_input_valid_failure', 'reject_input_overlap_success', 'reject_input_raster_pixel', 'reject_input_error_history', 'reject_events_expiry', 'reject_chord_missing_unit',
        'reject_cut_wrong_selection', 'reject_cut_wrong_block', 'reject_selection_lost_chord', 'reject_selection_vertical_field',
        'reject_select_horizontal_order', 'reject_select_vertical_projection', 'reject_scan_early_none', 'reject_scan_next_value',
        'reject_scan_lost_split', 'reject_scan_endpoint_split', 'reject_scan_missing_boundary', 'reject_output_aos_count', 'reject_output_packed_value',
        'reject_scratch_event_phase', 'reject_scratch_initial_output', 'reject_sink_count', 'reject_sink_error', 'reject_sink_argument'
    ) },
    @{ Name = 'matching'; Kind = '同源 CSR 与匹配核'; Cases = @(
        'reject_csr_order_duplicate', 'reject_csr_wrong_degree', 'reject_csr_pair_loss', 'reject_csr_source_frame',
        'reject_transpose_degree', 'reject_transpose_row_start', 'reject_transpose_edge', 'reject_match_nonedge', 'reject_match_occupied',
        'reject_match_flip_loss', 'reject_match_frame', 'reject_right_order_duplicate', 'reject_path_link', 'reject_path_count', 'reject_path_frame',
        'reject_bfs_seed', 'reject_bfs_depth', 'reject_bfs_missing_root', 'reject_dfs_path_depth', 'reject_dfs_root_seen', 'reject_dw_lost_path', 'reject_dw_lost_matching',
        'reject_maximum_empty', 'reject_maximum_conflict', 'reject_canonical_left_tie', 'reject_canonical_extra_right',
        'reject_sweep_closed_start', 'reject_sweep_closed_end', 'reject_sweep_lost_chain', 'reject_sweep_duplicate_edge'
    ) },
    @{ Name = 'geometry_optimal'; Kind = '完整像素域几何归约'; Cases = @('reject_corner_diagonal_owner', 'reject_corner_cross_weight', 'reject_corner_label_touch', 'reject_corner_double_count', 'reject_good_closed_endpoint', 'reject_vertical_closed_count', 'reject_lower_witness', 'reject_scan_close_order') },
    @{ Name = 'index'; Kind = '同源核'; Cases = @('reject_index_out_of_bounds', 'reject_index_narrowing') },
    @{ Name = 'geometry'; Kind = '同源核'; Cases = @('reject_shape_lod', 'reject_shape_alignment', 'reject_shape_bounds', 'reject_cell_end', 'reject_coord_end') },
    @{ Name = 'fixed'; Kind = '同源核'; Cases = @('reject_fixed_full_push', 'reject_fixed_clear_contents', 'reject_fixed_truncate_growth') },
    @{ Name = 'search'; Kind = '同源核'; Cases = @('reject_search_distance', 'reject_search_missing_free', 'reject_search_capacity', 'reject_search_invalid_id', 'reject_search_filtered_loss', 'reject_search_filter_order') },
    @{ Name = 'chord_graph'; Kind = '同源核及证书定理'; Cases = @('reject_chord_endpoint', 'reject_mark_matched_index', 'reject_uncovered_matching', 'reject_csr_degree_loss', 'reject_csr_unused_start', 'reject_scatter_capacity', 'reject_scatter_prefix') },
    @{ Name = 'packed'; Kind = '同源核'; Cases = @('reject_packed_clear_contents', 'reject_packed_full_push', 'reject_packed_iter_order', 'reject_packed_iter_length') },
    @{ Name = 'partition'; Kind = '规范推论'; Cases = @('reject_partition_overlap', 'reject_partition_background') }
)
$summary = @()
$status = 'failed'
$current = 'version'
try {
    $version = Invoke-CheckedProcess -FilePath $compiler -ArgumentList @('--version') -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'version')
    $reported = [IO.File]::ReadAllText($version.StdoutPath) + [IO.File]::ReadAllText($version.StderrPath)
    if ($reported -notmatch ('Version:\s*' + [regex]::Escape($tools.verus.version) + '(\s|$)')) { throw 'Verus 版本与 scripts/tools.json 不符' }
    $current = 'std'
    $null = New-Item -ItemType Directory -Path $adapter -Force
    $trusted = Invoke-CheckedProcess -FilePath $compiler -ArgumentList @('--edition=2024', '--rlimit', '10', '--triggers-mode', 'silent', '--compile', '--crate-type', 'lib', '--crate-name', 'rectangle_std', '--export', $metadata, '-o', $library, 'crates/rectangle_decomposition/verify/std.rs') -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs 'std')
    $trustedResult = [IO.File]::ReadAllText($trusted.StdoutPath) + [IO.File]::ReadAllText($trusted.StderrPath)
    if ($trustedResult -notmatch 'verification results::\s*([1-9][0-9]*) verified, 0 errors') { throw '标准库补充规格的位和证明失败' }
    $summary += [pscustomobject]@{ Target = 'std'; Kind = '位和证明；另含 6 项具体可信绑定'; Verified = [int]$Matches[1]; Seconds = $trusted.ElapsedSeconds; Rejected = 0 }
    Write-Host '可信补充：u64/u128 count_ones、u16 fill、64×64 u16 flatten、32-lane u16 SIMD 比较、单次 FnMut dispatch；不把绑定算作其实现证明。'
    foreach ($target in $targets) {
        $current = $target.Name
        $source = if ($target.Source) { $target.Source } else { "crates/rectangle_decomposition/verify/$current.rs" }
        $arguments = @('--edition=2024', '--no-cheating', '--rlimit', '10', '--triggers-mode', 'silent', '--import', "rectangle_std=$metadata", '--extern', "rectangle_std=$library")
        if ($target.Args) { $arguments += $target.Args }
        $arguments += $source
        $positive = Invoke-CheckedProcess -FilePath $compiler -ArgumentList $arguments -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix (Join-Path $logs $current)
        $result = [IO.File]::ReadAllText($positive.StdoutPath) + [IO.File]::ReadAllText($positive.StderrPath)
        if ($result -notmatch 'verification results::\s*([1-9][0-9]*) verified, 0 errors') { throw "未得到非空且无错误的 Verus 结果：$current" }
        $verified = [int]$Matches[1]
        $rejected = 0
        foreach ($case in $target.Cases) {
            $current = $case
            $prefix = Join-Path $logs $case
            try {
                $null = Invoke-CheckedProcess -FilePath $compiler -ArgumentList ($arguments + @('--cfg', $case)) -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix $prefix
                throw "负例未被拒绝：$case"
            } catch {
                if ($_.Exception.Data['ExitCode'] -ne 1 -or $_.Exception.Data['TimedOut']) { throw }
                $diagnostic = [IO.File]::ReadAllText("$prefix.stdout.log") + [IO.File]::ReadAllText("$prefix.stderr.log")
                if ($diagnostic -match 'resource limit|rlimit.*exceeded|timed out|error\[E[0-9]+\]|internal (compiler )?error|thread .*panicked') { throw }
                if ($diagnostic -notmatch 'verification results::\s*[0-9]+ verified, [1-9][0-9]* errors' -or $diagnostic -notmatch 'precondition not satisfied|assertion failed') { throw }
                $rejected++
                Write-Host "已拒绝：$case"
            }
        }
        $summary += [pscustomobject]@{ Target = $target.Name; Kind = $target.Kind; Verified = $verified; Seconds = $positive.ElapsedSeconds; Rejected = $rejected }
    }
    $status = 'passed'
    $summary | Format-Table -AutoSize
    Write-Host "矩形分解同源调用链、最优性及全部负例通过。日志：$logs"
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    [Console]::Error.WriteLine("失败入口：$current；日志：$logs")
    $code = $_.Exception.Data['ExitCode']
    if ($null -eq $code) { $code = 1 }
    exit [int]$code
} finally {
    if (Test-Path -LiteralPath $logs) {
        [pscustomobject]@{ Status = $status; Current = $current; Targets = $summary } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $logs 'summary.json') -Encoding UTF8
    }
}
