# Nsight GPU Trace 离线读取

[nsight-trace.py](../scripts/nsight/nsight-trace.py) 在 CPU 上只读已有 `.ngfx-gputrace`，不启动 Nsight、游戏、回放或 GPU。原始 trace、冻结二进制／SPIR-V、场景参数和分析结果保存在 `temp/`；每次使用新的或空的输出目录，工具拒绝覆盖证据。抓取文件可能含私有路径、源码与调试信息，发布前自行审查。

不随工具分发 NVIDIA descriptors、capture、shader/debug 数据或 XOR 映射。

## 环境与运行

从仓库根目录执行；按[本机配置](local.md)设置 `NSIGHT_HOST` 指向 Nsight 的 host 目录。先创建仓库内的可再生 venv：

```powershell
python -m venv target/nsight-python
$readerPython = 'target/nsight-python/Scripts/python.exe'
& $readerPython -m pip install -r scripts/nsight/nsight-requirements.txt
& $readerPython -B -m unittest discover -s scripts/nsight -p 'nsight*tests.py'
```

Linux 的解释器路径为 `target/nsight-python/bin/python`。缺少可选依赖而 skip 的测试不算解析验证通过。`--nsight-host` 必须显式提供，例如本机路径：

```powershell
$readerPython = 'target/nsight-python/Scripts/python.exe'
. ./scripts/load_local.ps1
$nsightHost = $env:NSIGHT_HOST
& $readerPython scripts/nsight/nsight-trace.py temp/captures/example.ngfx-gputrace `
    --out temp/nsight/example --nsight-host $nsightHost --max-chunk-mib 768 --pc-top 32
$readerExit = $LASTEXITCODE
```

该路径只是本机示例，不是默认值。工具从 `Plugins/WarpVizPlugin/WarpVizPlugin.dll` 文件字节提取 descriptors，不加载插件。已有五份 `.proto.pb` 时可改用 `--schema-dir temp/nsight-schema`；同时提供时优先使用该目录。两者都不提供仍保存 chunks，schema 结果为 unavailable。完整 counter 求值另需 `--nsight-host` 中受支持的 Windows x64 graphics host NVPerf；CPU 子进程不创建 target/device 或 sampling session。

`--max-chunk-mib` 默认 512 MiB。较大 trace 的单块可能超过该限制；示例显式放宽至 768 MiB，不改变工具默认值。按实际输入及可用内存设置，并保留该参数。

| 退出码 | 含义 |
|---|---|
| `0` | 所请求的基础分析完整 |
| `3` | partial：至少一项缺失／不支持，或显式使用 `--metadata-only`；可用结果保留 |
| `2` | 输入损坏、容器不支持、完整性检查失败或 CLI 错误 |

先检查 `summary.json` 及各组件的 `status`／`reason`，不能把 partial 当作全部通过。当前仅支持经过审查的 WRPV v10 容器及 descriptor SHA profile；未知版本明确失败，未知 descriptor 保留原始 chunks 与原因，不猜测兼容。

## 输出与统计边界

`summary.json` 的 `outputSchemaVersion=1` 记录输入分析前后 SHA256、工具／依赖／producer／host 证据、状态和限制。字段含义或单位改变须升级 schema，不能混用不同口径。

| 产物 | 可用证据 |
|---|---|
| `chunks.json`、`chunks/` | 压缩布局、flags、大小、解压 SHA256 与原始字节 |
| `schemas/manifest.json` | schema 来源、offset、descriptor hash 与支持 profile |
| `metadata.json` | 原始 InfoTables 键值、producer、采集设置 |
| `gpu-windows.json` | GPU frame 及 Dispatch／TraceRays 的 Ptimer 区间，单位 ns；保留 queue／stream／call／pipeline 和原始参数 |
| `counters.json`、`counter-samples.csv` | metric 名称、求值状态、chip、原值、有效覆盖及窗口汇总 |
| `shaders.json`、`shaders/` | SPIR-V／普通 shader／RT CUBIN、source SHA、ELF 函数地址、窗口内活跃 PC samples 与工具日志 |
| `report.md`、`sass-analysis.json` | 由上述证据生成的事实报告、静态 opcode 数量及精确 PC 指令对应；不自动判定瓶颈 |
| `source-analysis.json`、`sources/` | 显式模块配对下的高级调试位置、内联链与 PC 样本覆盖 |
| `internal-rt.json`、`internal-rt/` | 可选的 redacted RT ELF、具名绝对 VA 范围和独立 PC 归属 |

GPU call 区间要求 `NextCallIndex=ci/ci+1` 的唯一合法 GPU timestamp，不使用 CPU API duration。重复、缺失或越界单列诊断；`containedFrameIds` 区分帧内和邻近调用。原始 Dispatch 参数保持未独立验证，不按 shader 预期纠正。不同队列可重叠，不能把调用墙钟直接相加为整帧工作量。

指标按 periodic sample 与窗口的 overlap ns 汇总：百分比按 overlap 加权；`.sum` 按 `value × overlap / sampleDuration` 分摊，假设样本内部均匀，因此可以出现小数。查看 `validDurationNs`、有限样本数及 min/max；覆盖不足不外推，重叠样本不重复计覆盖。缺失、NaN 或失败保留 null，不能代替实际零。未知名称不猜单位；`--metric` 可重复指定精确 metric 名称。

默认包括 `pcie__read_bytes.sum`／`pcie__write_bytes.sum`，按上述窗口口径输出 bytes；read/write 保留指标原名，不擅自等同 NVML TX/RX。采集计数包含采集器及其他 GPU 活动的流量：Real-Time Shader Profiler 和 perf counters 本身消耗 PCIe，GPU Trace 也占用记录内存。PCIe 饱和不能单独归因应用资源；必要时关闭 Shader Profiler，在相同场景、时钟和设置下对照。[NVIDIA GPU Trace 说明](https://docs.nvidia.com/nsight-graphics/UserGuide/gpu-trace-overview.html)

`threadInstOver32WarpInst` 只使用 warp/thread 两项同时有限的 paired samples，保留 paired 覆盖；它不等于逐样本 lane 百分比的时间均值，也不能替代 resident occupancy。寄存器数、LSU 利用率、stall、流量和带宽分别解释；单个指标不能证明 spill 或整帧收益。

## Shader、PC 与可选工具

未混淆 shader 直接验证。`OBFUSCATION_SIMPLE` 默认保存 encoded 数据并标 unavailable；需要恢复时显式提供独立已识别的参考 capture/blob 和其原始 SPIR-V：

```powershell
& $readerPython scripts/nsight/nsight-trace.py temp/captures/target.ngfx-gputrace `
    --out temp/nsight/target --nsight-host $nsightHost `
    --xor-reference-trace temp/captures/independent-reference.ngfx-gputrace `
    --xor-reference-device 0 --xor-reference-blob 0 `
    --xor-reference-spv temp/reference/original.spv `
    --expected-spv-dir temp/target-build/spirv --cuda-bin '<CUDA安装目录>/bin'
```

XOR 映射只覆盖参考长度，更长目标明确 unavailable，不截断或用待验证 source 补尾。参考自身／同一 encoded/plain 对标记 `derivedFromReference`／`identityNotIndependentlyVerified`，不算独立身份确认；其他 blob 须全长解码、结构有效并与独立冻结 SPIR-V 的完整 SHA256 相等才进入 `sourceMatches`。身份匹配不证明实际 specialization、SPP、预算、种子或场景。

RT 用户程序取自 `rtCoreUserCubin`；普通 codeBlocks 可能只是宿主 VS/PS。PC 按原始 ELF function GPU VA 和半开 timestamp 窗口映射，资源结论须引用该窗口非零 samples 的 `activeFunctions`，inactive variant 单列。当前只解码显式 PerSM version 1 的 16-byte PC/timestamp 记录；其他或缺失版本保持 unavailable。

`--internal-rt` 另行导出 `rtCoreInternalRedactedCubin` 的具名 STT_FUNC：只接受已观察的 ET_EXEC、绝对 VA、空名零长 SHT_NOBITS 布局；缺失代码不反汇编或推断 REG。相同地址/长度的别名合并且保留来源，名称冲突、非相同范围交叠及与旧函数相交均标 ambiguous。仅支持单 device/GPU 地址域，按 `topPcs` 报告样本覆盖，不外推前 N 项。`allPcsExported` 仅表示全部导出，仍可能有未知身份；`attributionStatus=complete` 还要求全部样本唯一落入已有函数或内部具名范围。该 artifact 不改变原有 `functions` 索引或 `unmappedSamples`；符号名证明捕获中的类别归属，不证明内部算法、指令、流量或耗时。接口见 [internal_rt.py](../scripts/nsight/nsight_diagnostics/internal_rt.py)。

`--pc-top N` 可选按原 GPU 半开窗口统计精确 `(pc,rawSecondU64)` 的样本频次 `top`，并用 `topPcs` 跨 raw 汇总 PC；两者均取前 N 项。默认 0，不分配 histogram。PC 与第二个 u64 保留为 16 位十六进制，附已知 `functionIndex` 和函数相对 offset。默认不解释第二个 u64；具名 reason 须显式启用下述选项。`--max-pc-keys` 默认 1000000，限制跨全部 GPU 窗口的唯一 `(windowId,pc,raw)` 总项数；PC 汇总临时容量不超过当前窗口的 pair 数。超限时可选 histogram 标 unavailable，整体为 partial；保留原有 `activeFunctions` 和导出数据，不将截断数据伪装成 top N。频次及样本比例不等于耗时。

PC samples 不是 invocation、动态 instruction 数或精确耗时份额；等待位置可能是较早指令的消费者。unmapped 不全归 traversal。没有可信、版本明确的私有 packed reason 映射时，stall 分类保持 unknown，不拿 catalog index 或旧枚举命名 NoInst；缺少丢样字段也不能证明没有丢样。

`--pc-reasons` 需 `--nsight-host`，仅在 [pc_reasons.py](../scripts/nsight/nsight_diagnostics/pc_reasons.py) 的已审查 profile 下开启：安装版 `WarpVizPlugin.dll` SHA256 必须为 `e6ea6618cbf9be0c5ee143a1213159a9130f94b34347243275db8afb0437aa12`，且捕获 `Arch=400`、`Impl=2`、PerSM `version=1`。依据是该二进制的 wire 字段、原样复制及按架构 remap 的完整代码链；输出 `evidence` 记录其 RVA。产品名称、截图数值及字符串 catalog 的排列都不作为版本判断。`pcReasons.counts` 使用整个窗口，不依赖 `--pc-top`；`topPcs.reasonSamples` 则只覆盖所导出的 PC。raw code 未知时保留原值并标 partial，不把它记为零。

该 profile 只解释第二个 u64 的 bits 40..47。原始值和 raw reason code 继续保留；8／9 同名 Long Scoreboard、14／16 同名 MIO Throttle，并不因此丢掉原 code 或推断 RTCORE／L1TEX 子类。其他位、越界 code、未审查的版本及架构保持 unknown。这是有限的私有布局适配，不是 NVIDIA 的公开文件格式保证；reason 仍表示采样位置的 warp 状态，不能直接当作耗时或访存生产者。语义参考 [NVIDIA Shader Profiler](https://docs.nvidia.com/nsight-graphics/UserGuide/shader-profiler.html#stall-reasons)。

`--cuda-bin` 可选调用 CPU `cuobjdump`／`nvdisasm`，不需要 CUDA GPU 会话；保存工具 hash、version、stdout/stderr，非零退出或空 stdout 不算成功。派生 ELF 只在已验证布局下调整 symbol offset，核实 `.text` 字节不变，原件与 PC 映射继续使用 absolute VA。REG／CUDA STACK／LOCAL／SHARED 是静态 program metadata，不是 peak-live registers、整个 RT pipeline allocation 或动态 continuation／spill 字节；私有 `.rt.info.*` 仅记录大小，不解码未知 ABI，不能以 CUDA STACK=0 推出 RT 栈或 spill 为零。

报告只索引输出目录内、每份不超过 64 MiB 的成功 nvdisasm stdout；缺失、超限或未覆盖的指令标 unavailable，不改原始日志。`sourceFile`／`sourceLine` 保留工具标注，可能指向 SPIR-V 汇编而非 Slang；PC 必须恰好匹配指令起点，不向邻近指令猜测。`opcodeCounts` 是静态数量，不能代替动态请求、流量或耗时。报告的索引诊断与原始分析状态分别保留；缺失数据和 partial 均不代表零或完整验证。

## 高级源码关联

在 shader 解码与 `nvdisasm` 成功后，使用 `--spirv-dis '<VulkanSDK>/Bin/spirv-dis.exe' --source-pair 0:9=4`。配对语法为 `device:cubinModule=spirvModule`，可重复；索引取自本次 `shaders.json`，不能沿用其他 capture 的编号。`--pc-top` 决定可关联的 PC 范围，`allPcsExported` 仅在导出 PC 样本总和等于窗口总样本时成立。小 N 的排行榜不代表完整源码分布。

当前只接受 `NonSemantic.Shader.DebugInfo.100`。解析 `spirv-dis` 默认五行头与逻辑指令行，将多行 `OpString` 归为一条逻辑行；用 SASS 首条标注的 SPIR-V `OpFunction` 行检查锚点，随后只接受同一 IL 文件的精确行号。锚点证明行号约定一致，不代替模块身份验证。工具保存反汇编器身份、原始文本与诊断，不读取调试路径指向的外部源码。

高级行位置和函数作用域分别计覆盖：`DebugNoScope`／`DebugNoLine` 的缺失保留 unknown，不能用邻近指令或外层调用者补齐。内联链仅记录标准调试信息明确给出的帧；链不完整时不猜外层调用关系。函数按声明文件和 DebugFunction ID 区分重载；按行归属的 PC 样本仍不是 invocation、动态指令数或耗时。标准语义见 [Khronos DebugInfo](https://github.khronos.org/SPIRV-Registry/nonsemantic/NonSemantic.Shader.DebugInfo.html)。
