# GPU 进程超时

日常使用 `scripts/check.ps1 -Suite compute` 或 `-Suite rt`；需要直接运行其他 suite/bench 时，可在 Windows PowerShell 5.1 或 PowerShell 7（含 Linux）中调用通用运行器：

```powershell
. ./scripts/process.ps1
Invoke-CheckedProcess -FilePath cargo -ArgumentList @('test', '-p', 'shader_tests', '--test', 'shaders', '--', '--ignored') -TimeoutSeconds 300 -LogPrefix temp/gpu-tests/run
```

stdout/stderr 分别写入 `<LogPrefix>.stdout.log` / `.stderr.log`，结束后回放；进程树终止记录在 `.kill.log`。使用新的 LogPrefix 保留每次现场，不要把不可再生的失败日志放入 `target/`。

```powershell
Get-Help Invoke-CheckedProcess -Full
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/tests/process.ps1
```

调用成功返回退出码、用时和日志位置；子进程失败或超时会抛异常。调用脚本从 `Exception.Data['ExitCode']` 获取退出码，超时为 124，应向外层返回失败。进程树与日志排空的具体边界见源文件的帮助注释。

超时终止不保证失效驱动立即恢复，不继续复用原 GPU 环境。保留日志并按设备状态决定恢复方式；受测程序应等待自己的工作子进程结束。

A/B 测量可传 `scripts/gpu/bench_gpu.ps1 -TimeoutSeconds 300`，分别限制每个受测进程；validation 环境仍在清理阶段恢复。
