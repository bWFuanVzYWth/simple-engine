<#
.SYNOPSIS
有界执行一个 suite/bench 进程，分别保存并回放 stdout/stderr。
.DESCRIPTION
截止时间包含日志排空；超时终止进程树，无法保证失效驱动立即释放资源。
受测根进程须等待其子进程结束；根进程提前退出留下的孤立后代不在回收范围内。
.PARAMETER TimeoutSeconds
正常执行与管道排空的总秒数，清理另有短暂上限；超时退出码为 124。
.PARAMETER LogPrefix
日志前缀；覆盖同名 stdout/stderr，终止失败另存 kill 日志。
需保留的运行现场应使用 temp 下的新前缀，生成式自测可使用 target。
.OUTPUTS
成功返回 ExitCode、ElapsedSeconds、StdoutPath、StderrPath；失败抛异常，
Exception.Data 保留 ExitCode、TimedOut 和日志路径，调用方应传播失败。
#>
function Invoke-CheckedProcess {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$FilePath,
        [AllowEmptyCollection()][AllowEmptyString()][string[]]$ArgumentList = @(),
        [ValidateRange(1, 86400)][int]$TimeoutSeconds = 300,
        [string]$WorkingDirectory = (Get-Location).Path,
        [Parameter(Mandatory = $true)][ValidateNotNullOrEmpty()][string]$LogPrefix
    )

    $prefix = [IO.Path]::GetFullPath($LogPrefix)
    $null = [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($prefix))
    $stdoutPath = "$prefix.stdout.log"
    $stderrPath = "$prefix.stderr.log"
    $stdout = [IO.File]::Create($stdoutPath)
    $stderr = [IO.File]::Create($stderrPath)
    $process = New-Object Diagnostics.Process
    $process.StartInfo.FileName = $FilePath
    if ($process.StartInfo.PSObject.Properties['ArgumentList']) {
        foreach ($argument in $ArgumentList) { $process.StartInfo.ArgumentList.Add($argument) }
    } else {
        # Windows PowerShell 5.1 没有 ArgumentList，按 Windows argv 规则转义。
        $process.StartInfo.Arguments = (@($ArgumentList | ForEach-Object {
            $escaped = [regex]::Replace($_, '(\\*)"', '$1$1\"')
            '"' + [regex]::Replace($escaped, '(\\+)$', '$1$1') + '"'
        }) -join ' ')
    }
    $process.StartInfo.WorkingDirectory = [IO.Path]::GetFullPath($WorkingDirectory)
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.CreateNoWindow = $true
    if ($env:OS -eq 'Windows_NT') { $process.StartInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden }
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $timedOut = $false
    $exitCode = 1
    $copies = @()
    $cleanupError = $null
    try {
        if (-not $process.Start()) { throw "无法启动进程：$FilePath" }
        $copies = [Threading.Tasks.Task[]]@(
            $process.StandardOutput.BaseStream.CopyToAsync($stdout),
            $process.StandardError.BaseStream.CopyToAsync($stderr)
        )
        $milliseconds = $TimeoutSeconds * 1000
        $remaining = [Math]::Max(0, $milliseconds - [int]$clock.ElapsedMilliseconds)
        $timedOut = -not $process.WaitForExit($remaining)
        if (-not $timedOut) {
            $remaining = [Math]::Max(0, $milliseconds - [int]$clock.ElapsedMilliseconds)
            $timedOut = -not [Threading.Tasks.Task]::WaitAll($copies, $remaining)
        }
        if ($timedOut) {
            # 杀整个 suite/bench 进程树，不能取消一个 case 后复用失效设备。
            try {
                if ([Diagnostics.Process].GetMethod('Kill', [type[]]@([bool]))) {
                    $process.Kill($true)
                    [IO.File]::WriteAllText("$prefix.kill.log", 'Process.Kill(entireProcessTree: true)')
                } else {
                    $killer = New-Object Diagnostics.Process
                    try {
                        $killer.StartInfo.FileName = Join-Path $env:SystemRoot 'System32/taskkill.exe'
                        $killer.StartInfo.Arguments = "/PID $($process.Id) /T /F"
                        $killer.StartInfo.UseShellExecute = $false
                        $killer.StartInfo.CreateNoWindow = $true
                        $killer.StartInfo.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
                        $killer.StartInfo.RedirectStandardOutput = $true
                        $killer.StartInfo.RedirectStandardError = $true
                        $killer.StartInfo.StandardOutputEncoding = [Text.Encoding]::Default
                        $killer.StartInfo.StandardErrorEncoding = [Text.Encoding]::Default
                        $null = $killer.Start()
                        $killOut = $killer.StandardOutput.ReadToEndAsync()
                        $killErr = $killer.StandardError.ReadToEndAsync()
                        if (-not $killer.WaitForExit(5000)) {
                            $killer.Kill()
                            throw '终止进程树超时'
                        }
                        [IO.File]::WriteAllText("$prefix.kill.log", $killOut.Result + $killErr.Result)
                        if ($killer.ExitCode -ne 0) { $cleanupError = '终止进程树失败，见 kill.log' }
                    } finally {
                        $killer.Dispose()
                    }
                }
            } catch {
                $cleanupError = $_.Exception.Message
            }
            if (-not $process.WaitForExit(5000)) { $cleanupError = '终止后仍无法确认主进程退出' }
            try {
                if (-not [Threading.Tasks.Task]::WaitAll($copies, 5000)) { $cleanupError = '终止后日志管道仍未关闭' }
            } catch {
                $cleanupError = $_.Exception.Message
            }
            $exitCode = 124
        } else {
            $exitCode = $process.ExitCode
        }
    } catch {
        $_.Exception.Data['ExitCode'] = if ($timedOut) { 124 } else { 1 }
        $_.Exception.Data['TimedOut'] = $timedOut
        throw
    } finally {
        # 子进程继承管道后也受总截止时间约束，不无限等待日志 EOF。
        if ($copies.Count -ne 0) {
            $process.StandardOutput.Dispose()
            $process.StandardError.Dispose()
        }
        $stdout.Dispose()
        $stderr.Dispose()
        $process.Dispose()
        $clock.Stop()
    }
    [Console]::Out.Write([IO.File]::ReadAllText($stdoutPath))
    [Console]::Error.Write([IO.File]::ReadAllText($stderrPath))
    if ($exitCode -ne 0) {
        $message = if ($timedOut) { "进程超过 ${TimeoutSeconds}s：$FilePath" } else { "进程退出 ${exitCode}：$FilePath" }
        if ($cleanupError) { $message += "；$cleanupError" }
        $failure = New-Object Exception "$message；日志 $prefix.*.log"
        $failure.Data['ExitCode'] = $exitCode
        $failure.Data['TimedOut'] = $timedOut
        $failure.Data['StdoutPath'] = $stdoutPath
        $failure.Data['StderrPath'] = $stderrPath
        throw $failure
    }
    [pscustomobject]@{
        ExitCode = $exitCode
        ElapsedSeconds = $clock.Elapsed.TotalSeconds
        StdoutPath = $stdoutPath
        StderrPath = $stderrPath
    }
}
