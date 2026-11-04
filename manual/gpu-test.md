# GPU 测试、失败重放与测量

优先用带外层超时的检查档，失败日志保存在 `temp/check/`：

```powershell
powershell -NoProfile -File scripts/check.ps1 -Suite compute
powershell -NoProfile -File scripts/check.ps1 -Suite rt
```

检查档内容与依赖见[开发脚本](../scripts/README.md)。以下底层命令方便筛选，不提供外层进程超时。设备初始化失败算失败，未运行项不能算通过。

## 运行与筛选

```powershell
cargo test --locked -p gpu_test --lib
cargo test --locked -p shader_tests --test shaders -- --list
cargo test --locked -p shader_tests --test shaders -- --ignored "[compute]"
cargo test --locked -p shader_tests --test shaders -- --ignored "[rt]"
cargo test --locked -p shader_tests --test display -- --ignored --test-threads=1
cargo test --locked --release -p openpbr_tests -- --ignored --test-threads=1
cargo test --locked --release -p atmosphere --lib -- --ignored --test-threads=1
cargo test --locked --release -p shader_tests --test atmosphere -- --ignored --test-threads=1
cargo test --locked --release -p wgpu_vulkan_tests --test sampled -- --ignored --test-threads=1
cargo test --locked -p wgpu_vulkan_tests --test interop -- --ignored --test-threads=1
```

shader 用例默认 ignored，需要时显式启用；互操作窗口项还需要桌面环境。`libtest-mimic` 直接输出 stdout/stderr，设施清理时报告能力环境的累计成本。采样和表面属性默认各生成 65536 个输入，可覆盖规模与完整 seed：

```powershell
$env:GPU_TEST_CASES = "262144"
$env:GPU_TEST_SEED = "0000000000000000000000000000000000000000000000000000000000000000"
cargo test --locked -p shader_tests --test shaders -- --ignored "[compute]"
```

seed 接受按字节顺序书写的 64 个十六进制字符。稳定随机协议见 [`gpu_test::input`](../target/doc/gpu_test/input/index.html)，输入 shape 与具体 schema 见[属性输入](../target/doc/shader_tests/input/index.html)。环境复用、失效与析构检查见[测试设施 API](../target/doc/gpu_test/index.html)。

## 失败重放与入库

性质失败默认写入 `temp/shader-tests/failures/<case>-<index>-<input_hash>-<time>.json`，避免覆盖旧现场。先保留原记录和相关日志，使用完整输入重放：

```powershell
$record = (Get-ChildItem temp/shader-tests/failures/*.json | Select-Object -First 1).FullName
cargo run --release -p shader_tests --bin shader_bench -- --replay $record --output temp/shader-tests/replay.json
```

输出包含当前结果和原/当前 shader 指纹；仍违反性质时进程返回非零。记录格式、位模式与 schema 分工见 [`Failure`](../target/doc/gpu_test/record/struct.Failure.html)，重放支持范围见 [`run::replay`](../target/doc/shader_tests/run/fn.replay.html)。

先确认失败能否脱离原批次重现，再在保持性质前提的情况下缩小输入。涉及 dispatch 位置或并发顺序时保留完整上下文。审查后的失败 JSON 复制到 `tests/shaders/tests/regressions`，逐文件注册为 ignored GPU 回归，修复后继续保留；`GPU_TEST_CORPUS` 可替换输入目录。

validation error、device lost 或超时后停止复用该环境。GPU hang 由[外层进程超时](gpu-runner.md)处理；保留现场后再决定重启进程或恢复设备。

## 定向测量

当前 CLI 测量生产 geometry 的三角端点与连接构造，不含硬件求交。按需选择 `all`、`coherent`、`incoherent`、`grazing` 分布及坐标尺度：

```powershell
cargo run --release -p shader_tests --bin shader_bench -- --count 262144 --dispatches 256 --rounds 7 --distribution all --scale 1
```

默认报告存入 `temp/shader-tests/bench-<time>.json`；`--output path.json` 可指定保存位置，`--seed HEX64` 固定输入。样本单位与 GPU/host/prepare 计时边界见 [`Sample`](../target/doc/gpu_test/struct.Sample.html)和[测量入口](../target/doc/gpu_test/struct.Gpu.html#method.measure)。

运行时大气分别测初始化求解、观察者更新、仅视锥更新与稳态空派发；初始化含八轮多重散射和完成等待，不把它混入每帧查表成本：

```powershell
cargo bench --locked -p atmosphere --bench runtime
```

计时边界见各 bench 的模块文档。算法新增会改变积分语义，不能把与旧 Lambert／渐变天空的帧时间差称为等价回归。

A/B 比较先分别保存两个版本的 release 可执行文件，保存在 `temp/` 避免后续构建覆盖：

```powershell
powershell -NoProfile -File scripts/gpu/bench_gpu.ps1 -Before temp/shader-bench-before.exe -After temp/shader-bench-after.exe -Pairs 7
```

脚本临时关闭 debug/validation，结束后恢复；交替运行顺序，核对设备、驱动与输入身份。默认保存到 `temp/gpu-ab/<unique>/`，包含原始 JSON、`pairs.csv` 和 `summary.json`，可用 `-Output` 指定新目录。比较时同时保存版本、工具链、负载和工作集信息，统计结果用于分析，不自动判定性能回归。
