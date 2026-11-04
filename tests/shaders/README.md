# Shader 检验与测量

需 native Vulkan 设备和 Slang，直接运行生产 SPIR-V。完整检查档、超时与 A/B 操作见[GPU 手册](../../manual/gpu-test.md)。

```powershell
cargo test --locked -p shader_tests --test shaders -- --list
cargo test --locked -p shader_tests --test shaders -- --ignored "[compute]"
cargo test --locked -p shader_tests --test shaders -- --ignored "[rt]"
```

用 `GPU_TEST_CASES` 调整属性批量，`GPU_TEST_SEED` 接收按字节顺序书写的 64 个十六进制字符；默认 seed 全零。小批量用于诊断，统计容限不会自动放宽。[批量、失败记录与统计约定](tests/shaders.rs)保留在测试入口；[输入 schema](../../target/doc/shader_tests/input/index.html)与[执行 API](../../target/doc/shader_tests/run/index.html)由 cargo doc 生成。

PT 固定 binary Sobol 消费域的数学契约与已知代价另有定向检查：

```powershell
cargo test --release --locked -p shader_tests --test binary_sobol
cargo test --release --locked -p shader_tests --test binary_sobol -- --ignored --test-threads=1
```

首条直接检查实际内嵌 SPIR-V 无索引循环、函数调用、宽算术或额外采样资源；第二条在真实 GPU 核对独立整数参考、net／定向投影及两点和三维 parity 缺陷。`scripts/check.ps1 -Suite compute` 同时运行这两类检查。测试不会把有限投影保证解释为场景或重建质量等价。

共享 PT 的双场景适配检查：`cargo test -p shader_tests --test pt_scene -- --ignored --test-threads=1`。使用不同属性来源翻译相同 OpenPBR 输入，核对 Beer、发光/MIS、介质边界与路径结果；同时与独立解析值对照。

通用降噪辅助量检查：`cargo test -p shader_tests --test denoise -- --ignored --test-threads=1`。
验证首命中与 delta landing 的区分、观察者不改变输运、OpenPBR 方向能量、无 jitter
像素运动与天空无平移视差；不依赖重建 SDK。

性质失败写入 `temp/shader-tests/failures`。保留原记录和日志，再重放实际输入：

```powershell
$record = (Get-ChildItem temp/shader-tests/failures/*.json | Select-Object -First 1).FullName
cargo run --release -p shader_tests --bin shader_bench -- --replay $record --output temp/shader-tests/replay.json
```

输出并列保留原／现 SPIR-V 指纹，仍违反性质时返回非零。确认可复现并审查后，把失败 JSON 放入 `tests/shaders/tests/regressions`；`GPU_TEST_CORPUS` 可替换回归目录。

定向负载可选择 `all|coherent|incoherent|grazing` 分布；`--count` 调整工作集，`--scale` 调整世界尺度，`--seed HEX64` 固定输入，`--adapter` 选择设备：

```powershell
cargo run --release -p shader_tests --bin shader_bench -- --count 262144 --dispatches 256 --rounds 7 --distribution all --output temp/shader-tests/bench.json
```

负载与计时边界见[测量入口](src/bin/shader_bench.rs)及 [`Gpu::bench_compute`](../../target/doc/gpu_test/struct.Gpu.html#method.bench_compute)。性能比较使用手册中的 A/B 脚本关闭 validation 并保存两版程序及原始报告，结果用于分析，不设回归门槛。

light BVH 常驻查询基线将 tree/query 分 buffer，分别测 sample 与 reverse PMF；默认
grid/clustered × 24/4096/65536 灯、两种查询顺序，共24项。`--case` 按完整 case 名的子串过滤：

```powershell
cargo run --release -p shader_tests --bin light_bvh_bench -- --case grid/4096/coherent --output temp/light-bvh/baseline
```

[入口](src/bin/light_bvh_bench.rs)记录 CPU build、分配、完成上传、encode/submit/completion/
timequery 与 GPU timestamp。树和查询在稳态轮次中不更新；完整输出回读、校验另计。
GPU 时间包含每次 dispatch 独立 pass 的 WAW 依赖。各入口重复记录共享的树构建／上传及
查询准备字段，不应逐 case 相加；稳态样本分别计时。
`--count 65536 --dispatches 8 --rounds 3` 为默认批量；重复查询是热工作集，不代表整帧或
单次函数延迟。coherent 每64项共享接收点，selector/反查 ID 仍变化；incoherent 仅打乱同一
查询集合，还原原序后输出必须逐位一致。新输出目录保存 quads/tree/queries、两入口结果
二进制与指纹。布局比较固定几何与查询，记录选灯和 PMF 的逐位差异，按数学契约审查。
timestamp 不支持时
显式用 `--timing host`，报告的 GPU 时间为 null；不会自动以主机耗时替代。
