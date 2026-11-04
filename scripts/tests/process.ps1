$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
. (Join-Path $PSScriptRoot '../process.ps1')
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$directory = Join-Path $taskRoot "target/process-tests/$([Guid]::NewGuid().ToString('N'))"
$null = [IO.Directory]::CreateDirectory($directory)
$shell = (Get-Process -Id $PID).Path
$utf8 = New-Object Text.UTF8Encoding $true
$child = Join-Path $directory 'child script.ps1'
[IO.File]::WriteAllText($child, @'
param([string]$Mode, [string]$Directory)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
if ($Mode -eq 'echo') {
    ConvertTo-Json -InputObject @($args) -Compress
    [Console]::Error.WriteLine('stderr 已保留')
    exit 0
}
if ($Mode -eq 'fail') { [Console]::Out.WriteLine('退出前的日志'); exit 7 }
if ($Mode -eq 'leaf') {
    [IO.File]::WriteAllText((Join-Path $Directory 'leaf.pid'), "$PID")
    Start-Sleep -Seconds 60
}
if ($Mode -eq 'branch' -or $Mode -eq 'tree') {
    [IO.File]::WriteAllText((Join-Path $Directory "$Mode.pid"), "$PID")
    $next = if ($Mode -eq 'tree') { 'branch' } else { 'leaf' }
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -Mode ' + $next + ' -Directory "' + $Directory + '"'
    $options = @{ FilePath = (Get-Process -Id $PID).Path; ArgumentList = $arguments; PassThru = $true }
    if ($env:OS -eq 'Windows_NT') { $options.WindowStyle = 'Hidden' }
    $null = Start-Process @options
    Start-Sleep -Seconds 60
}
'@, $utf8)

function Run-Child([string]$Mode, [string[]]$Extra = @(), [int]$Timeout = 15) {
    Invoke-CheckedProcess -FilePath $shell -ArgumentList (@(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', $child, '-Mode', $Mode, '-Directory', $directory
    ) + $Extra) -TimeoutSeconds $Timeout -WorkingDirectory $directory -LogPrefix (Join-Path $directory $Mode)
}

$result = Run-Child 'echo' @('带空格 value', 'quote"value', 'C:\tail\', '')
$actual = Get-Content -LiteralPath $result.StdoutPath -Raw -Encoding UTF8 | ConvertFrom-Json
if (($actual | ConvertTo-Json -Compress) -ne (@('带空格 value', 'quote"value', 'C:\tail\', '') | ConvertTo-Json -Compress)) {
    throw 'argv 转义未保留原始参数'
}
if ((Get-Content -LiteralPath $result.StderrPath -Raw -Encoding UTF8) -notmatch 'stderr 已保留') {
    throw 'stderr 未保存'
}

$caught = $false
try { $null = Run-Child 'fail' } catch {
    if ($_.Exception.Data['ExitCode'] -ne 7) { throw }
    $caught = $true
}
if (-not $caught) { throw '非零退出未传播' }
if ((Get-Content -LiteralPath (Join-Path $directory 'fail.stdout.log') -Raw -Encoding UTF8) -notmatch '退出前的日志') {
    throw '失败日志未保存'
}

$caught = $false
try { $null = Run-Child 'tree' @() 4 } catch {
    if ($_.Exception.Data['ExitCode'] -ne 124 -or -not $_.Exception.Data['TimedOut']) { throw }
    $caught = $true
}
if (-not $caught) { throw '超时未传播' }
foreach ($name in @('tree', 'branch', 'leaf')) {
    $pidPath = Join-Path $directory "$name.pid"
    if (-not (Test-Path -LiteralPath $pidPath)) { throw "$name 未实际启动" }
    $childId = [int](Get-Content -LiteralPath $pidPath -Raw)
    $childProcess = Get-Process -Id $childId -ErrorAction SilentlyContinue
    if (-not $childProcess) { continue }
    try {
        $settle = [Diagnostics.Stopwatch]::StartNew()
        while (-not $childProcess.HasExited) {
            # Linux 非直接子进程的 HasExited 也可能把已终止的 zombie 当存活。
            if ($IsLinux -and ((Get-Content -LiteralPath "/proc/$childId/status" -ErrorAction SilentlyContinue) -match '^State:\s+Z\b')) { break }
            if ($settle.Elapsed.TotalSeconds -ge 5) { throw "$name 进程 $childId 仍存活" }
            Start-Sleep -Milliseconds 50
        }
    } finally {
        $childProcess.Dispose()
    }
}
Write-Host "进程测试通过：$directory"
