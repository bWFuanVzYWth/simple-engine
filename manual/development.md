# 开发检查与维护

Nsight 已有 GPU Trace 的 CPU 离线读取使用 [nsight-trace.py](../scripts/nsight/nsight-trace.py)，依赖隔离安装到 `target/nsight-python`；命令、来源许可和指标边界见 [Nsight 手册](nsight.md)。它不启动捕获、回放或 GPU。

## 资源打包

在已安装项目 Rust 工具链的 Windows PowerShell 中，从仓库根目录执行下面的命令。示例将项目说明和许可各自使用 Zstandard 22 压缩成一个可按名称读取的 ZIP 条目，最后导回说明文件。

```powershell
cargo run --locked --release -p resource_pack --features cli -- pack target/example-resources.zip readme=README.md license=LICENSE
cargo run --locked --release -p resource_pack --features cli -- list target/example-resources.zip
cargo run --locked --release -p resource_pack --features cli -- get target/example-resources.zip readme target/readme-copy.md
Get-FileHash README.md, target/readme-copy.md
```

列表应有 `readme` 和 `license`，两份说明的散列值应相同。条目名使用正斜线相对路径，源文件和输出目录须存在。Rust 应用可通过引擎的 `resource_pack` 功能读写文件或内存包，并限制单个条目的展开大小。自制资源按使用方式存储：

| 资源 | 保存格式与用途 |
| --- | --- |
| 相机和瞳孔的二维衍射核 | 每张独立使用 KTX2、线性 RGB32F、Zstd 22，可直接由标准纹理工具打开。[衍射核工作台](../examples/diffraction_lab/README.md) |
| 大气的固定地球参数和四通道介质表 | `crates/atmosphere/data/earth.zip` 含两个定长缓冲条目，初始化时校验并解压到目标存储。它们由参数偏移寻址。[大气物理输入](../crates/atmosphere/data/README.md) |
| 光瞳参数、大气拟合与测试参考、宿主翻译和清单 | 保留 JSON，供编辑器、测试工具或宿主按字段读取。 |
| 快速圆形滤波权重、大气辐射场、示例场景和体素预览 | 按构建配方或在运行时生成，无需分发预生成纹理。 |

Prime PT 从引擎读取镜头标定和大气输入。衍射工作台可从 PNG 导入用户光瞳，配方保存为 JSON，计算后的核可直接导出为 KTX2。体素编辑器在运行时生成预览。游戏原型的九张方块贴图来自 Squareful-BE，来源和开发用途见[占位纹理清单](../app/game/assets/textures/README.md)。

## 检查入口

Windows PowerShell 5.1 或 PowerShell 7：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/check.ps1 -Suite cpu
```

Linux 使用 `pwsh -NoProfile -File scripts/check.ps1 -Suite cpu`。

| Suite | 实际执行 | 环境 |
|---|---|---|
| `cpu` | 引擎仓库 Rust/Slang 格式、基础库 CPU 测试、Clippy、进程超时自测 | Rust、Git Bash/Bash、clang-format；不需要 Slang 或 GPU |
| `features` | 根包默认、CPU、GUI、2D GPU、native 与 Web feature 组合检查，含 wasm | Rust、Slang；不需要 GPU |
| `docs` | native/wasm rustdoc、链接/HTML 检查、文档示例、公开项覆盖率 100% | 固定 nightly Rust、Slang；Windows 另需锁定 Streamline SDK |
| `compute` | shader compute、light BVH／采样、OpenPBR／大气／显示、原始 bits 重放、2D 像素 | Slang、Vulkan compute；timestamp 用例另需对应能力 |
| `rt` | RT 性质、计时、Cornell 光追、实例保活及互操作集成测试（含窗口 resize） | Slang、Vulkan KHR 光追、桌面环境 |

GPU 档显式开启 validation 与同步验证，结束后恢复环境。每个命令由外层子进程限时；`-TimeoutSeconds` 默认 900，超时退出 124，其余失败传播子退出码。日志保存在 `temp/check/<suite>/<run>`，完成一档不代表其它档通过。语义见 [进程超时](gpu-runner.md)。

CI 在 Linux 执行 `cpu` 与 `features`，在 Windows 执行 Cargo 构建检查和 `docs`。GPU 档在具备设备的环境中手动执行。性能测量独立运行。

首次执行 `features`、`docs` 或 Web 构建前运行 `rustup target add wasm32-unknown-unknown`。更换固定 nightly 后重新添加。Windows 上执行 `docs` 前按 [Streamline 手册](streamline.md) 安装锁定 SDK。

`docs` 在 `target/doc` 和 `target/wasm32-unknown-unknown/doc` 生成文档，`target/doc-coverage.json` 汇总当前 native host 与 wasm 的 rustdoc 公开项覆盖。统计会省略部分重导出项，缺失注释仍由整个 workspace 继承的 `missing_docs` 阻断；覆盖率不代表示例覆盖或语义质量。入口和操作见 [操作手册](README.md)。

功能裁剪使用 `-Suite features` 独立检查根包，避免 workspace 成员统一 feature 后掩盖缺失依赖；各开关语义见 [引擎入口](engine.md)。

Windows 上检查 Streamline 桥接的退出失败处理：先按 [DLSS RR 手册](streamline.md) 安装锁定 SDK，并安装带 `clang++` 的 LLVM；在仓库根目录的 PowerShell 运行下面命令。它使用模拟 SDK 调用，不创建窗口或 GPU 设备。成功会显示 `4 failure-boundary tests passed`；编译失败先检查 `clang++`、MSVC 工具链和 SDK 路径，断言失败保留完整终端输出。

```powershell
./crates/vulkan/streamline/test_bridge.ps1
```

## 工具链

在 Windows x64 上构建 NRD 降噪组件需要 PowerShell 7、Git、CMake、Visual Studio C++ x64 工具和带有 `Bin/dxc.exe` 的 Vulkan SDK。确认 `VULKAN_SDK` 指向安装目录，在引擎根目录的 PowerShell 中运行 `cargo check --locked -p vulkan --features nrd`。首次构建会在 `target/` 下载锁定版本的 [NRD](https://github.com/NVIDIA-RTX/NRD/tree/v4.18.0) 及其依赖，生成静态库和 SPIR-V。终端显示 `Finished` 即完成。若配置失败，可从 Cargo 输出检查 Git 下载、CMake 生成器或 DXC 路径。

本机路径、私有环境与凭据放在被忽略的 `local/`，模板、显式加载和 Cargo 覆盖方式见[本机配置](local.md)。

Rust 日期由 `rust-toolchain.toml` 固定。Slang 发行包与 clang-format 版本记录在 `tools.json`；CI 使用这些基线。本地在同一 PowerShell 会话中安装并构建：

```powershell
./scripts/install_slang.ps1
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets
cargo check --locked -p simple_engine --features webgpu,render2d_gpu --target wasm32-unknown-unknown
```

安装器校验官方 Slang ZIP 的 SHA256，解压到 `target/tools`，只设置当前进程的 `SLANGC`。Windows/Linux x86_64 提供固定发行包；其它平台自行设置 `SLANGC`。原有 SDK 和其它编译器仍可用，实际版本与组件身份进入构建记录，见 [shader 构建](shader-build.md)。

格式基线为 clang-format 23.1.3，可用 `python -m pip install clang-format==23.1.3` 安装。

VSCode 的 rust-analyzer 使用独立的 `target-ra/`，普通 Cargo 使用 `target/`。首次应用仓库配置后执行 `rust-analyzer: Restart server`，无需关闭 VSCode；之后 `cargo clean` 不会清理 IDE 正在加载的过程宏 DLL。代价是多一份可再生构建缓存；清理 `target-ra/` 前先执行 `rust-analyzer: Stop server`，清理后再 `Start server`。

## Verus 实验

验证入口直接装配生产同源模块，工具包版本与 SHA256 固定在 `tools.json`；
生产执行代码使用普通 Rust 与条件属性契约；普通构建使用固定 vstd 擦除证明连接，不运行证明器。
首次验证时安装独立的 Rust 1.99.1 和锁定的 Verus。验证入口自动发现 `local/tools/verus/` 中的安装。

```powershell
rustup toolchain install 1.99.1 --profile minimal
./scripts/verus.ps1 -Install
./crates/vulkan/verify/check.ps1
```

验证器以 `--no-cheating --rlimit 10` 检查下列全部入口及对应违规反例：

| 入口 | 同源范围与契约 |
|---|---|
| `arena` | [槽表](../crates/vulkan/src/arena.rs)：插入、查找、借用、移除、失效身份与终代停用；值游标精确跳过空槽，类型化身份由真实 `record_ids` 接入 |
| `resource` | [真实资源枚举](../crates/vulkan/src/resource.rs)：依赖/访问选择、producer 更新的完整 payload 帧与借用 Graph 对应；链接期望类别不替代目标实际类别检查 |
| `graph` | [依赖计划](../crates/vulkan/src/graph.rs)：完整 Key 校验、任意环的精确最小闭包、发布种子扩展及有界工作表；已标记槽只读身份，无出边节点即时标记，不复制全图 |
| `lifecycle` | [实际所有者](../crates/vulkan/src/lifecycle.rs)、[记录](../crates/vulkan/src/record.rs)、[批次](../crates/vulkan/src/record_batch.rs)及[根游标](../crates/vulkan/src/native_graph.rs)：完整身份、受限移出、producer、原值帧、整批准备/回滚/接受/隔离、完成水位、退役身份唯一；真实根序列、计划/提交失败原子性及现存记录可达依赖保留 |
| `retirement` | [退役分组](../crates/vulkan/src/retirement.rs)：精确有序过滤、存续 waits 不动、新组取实际记录身份；条件式唯一性/等待包含关系与 ready 组按值移出 |
| `submission` | [提交状态](../crates/vulkan/src/submission.rs)：故障终态、记录转换、无回绕序号与单调完成水位；不含整个宿主适配证明 |
| `idle` | [闲置核](../crates/vulkan/src/idle.rs)：真实 VecDeque 的相对顺序、最旧驱逐和预算谓词；不含完整 pool 字节求和与 FFI |
| `recovery` | [恢复核](../crates/vulkan/src/recovery.rs)：重配后保留 reset，成功录制后消费；不证明 SDK DLL 行为 |
| `lifetime` | [独立协议模型](../crates/vulkan/verify/lifetime.rs)，不计为生产实现证明 |

初始化、编译错误和超时不算负例通过。支持 `-Verus <path>` 与每次调用的 `-TimeoutSeconds`，
原始日志及 `summary.json` 写入 `temp/verus/<run>`。Windows 已实测；Linux 安装路径尚未验证。

各项规范和可信边界见所属源码，辅助证明集中在各 crate 的 `src/proof/`，作为所属实现的私有子模块装配。
局部证明不涵盖完整 Vulkan crate；普通 Cargo 检查也不会运行此入口，GPU/validation 用例仍需单独执行。
`collect` 从同一 Life 的真实根与邻接制定并提交计划；后续消费者准入、首次 waits 覆盖的跨调用归纳和 rank 销毁仍须审查。
条件式回收引理不等于完整 owner 不变量已闭合。固定 vstd 尚缺 fallible HashSet 预留和排序/去重规格，
producer 遍历及 uses 排序保留原实现并明确未证，不新增外部假设绕过严格入口。
依赖入口会重复检查共享模块，验证单元数不直接相加。
[官方安装](https://github.com/verus-lang/verus/blob/release/0.2026.10.26.d17fbd0/INSTALL.md)、
[可信组件](https://verus-lang.github.io/verus/guide/tcb.html)、
[后续 Cargo 集成](https://verus-lang.github.io/verus/guide/cargo_verus.html)。

SipHash 使用独立入口：

```powershell
./crates/sip_hash/verify/check.ps1
```

它装配 [SipHash](../crates/sip_hash/src/lib.rs)、标量与实际 `portable_simd` 内核，
覆盖任意有效切片、三坐标编码、逐 lane 等价、全部 `usize` 轮数及零轮。
固定 Verus 缺少的标准原语规范由 [可信适配](../crates/sip_hash/verify/std.rs) 单独编译，
不计为证明；生产同源入口保持 `--no-cheating --rlimit 10`，没有跳过整个 round/hash。
执行入口显式检查 `no_unwind`；运算原语与 native 共用最小适配函数，未假设整个内核不会 panic。
六个违规反例分别检查密钥字、坐标字序、缺失尾块、lane 顺序、零轮输出和 unwind 调用契约。
初始化、编译错误及超时均失败；日志和 `summary.json` 保存于 `temp/verus-sip-hash/<run>`，
支持相同的 `-Verus` 和 `-TimeoutSeconds`。Windows 已实测；wasm 仅普通 Cargo 检查，Linux 未实测。

HashDag 使用独立入口：

```powershell
./crates/hashdag/verify/check.ps1
```

入口装配 [HashDag](../crates/hashdag/src/lib.rs) 的实际节点、池、根槽、写入、回收与精确差分，
契约及可信边界见所属接口，复杂辅助集中在 `src/proof/`。`node`、`roots`、`storage`、
`canonical`、`write`、`diff`、`rc`、`owner` 八个入口全部检查，不使用局部过滤；共享模块不重复累计覆盖。
违规反例覆盖 mask、折叠、Morton 轴序、去重、认领回滚、根失效、重复边回收与差分集合等关键结论。

hashbrown 按可信容器处理；完整内容判等、候选哈希一致性、引用变化和事务算法仍由同源代码证明。
固定工具链尚缺的标准库／SIMD／分配／原子原语在[最小适配](../crates/hashdag/verify/std.rs)单独编译，
不计为证明。同一导出库中的[身份发行核](../crates/hashdag/src/identity.rs)另以 `OwnedIdentity` 阶段
检查全部模块及辅助证明；checked 递增、饱和、唯一票据与实际两次无参数构造的隔离均有证明。
固定 Verus 的 `--no-cheating` 会扫描整个 crate，不能排除其中的标准原语；此阶段审计发行源码
不含假设或跳过算法，并精确限制身份适配只允许原子初始化、线性化 RMW、同一 static 对象关联。
其余八入口使用 `--no-cheating --rlimit 10`。依赖由 `crates/hashdag/verify/Cargo.lock` 固定，
正式入口不依赖实验目录；编译错误、超时及非断言失败均不能充当反例通过。
原始日志和 `summary.json` 保存于 `temp/verus-hashdag/<run>`，支持相同的 `-Verus`、`-TimeoutSeconds`。
执行成本另作同源重构前后对照；形式化检查不替代 Cargo、平台检查或性能测量。

### 矩形分解

```powershell
./crates/rectangle_decomposition/verify/check.ps1
```

使用同一固定 Verus，算法入口均带 `--no-cheating --rlimit 10`，检查正式装配、生产调用链及违规反例。
`-Verus` 可指定安装路径，实际版本仍须匹配 `scripts/tools.json`。
先独立构建显式可信适配库，再供算法入口导入；导入不递归审计其可信绑定。
初始化、编译错误、资源超限及超时不算拒绝成功；`-Verus`、`-TimeoutSeconds` 的语义同上，
原始日志及包含成功／失败状态的 `summary.json` 写入 `temp/verus/rectangles/<run>`。

| 入口 | 覆盖 |
|---|---|
| `std`／`std_checks` | 有限位和证明及六项显式绑定：`u64`／`u128 count_ones`、具体 `u16 fill`、64×64 `u16 flatten`、32-lane `u16` SIMD 比较、单次 `FnMut` dispatch；不计为其实现证明 |
| `implementation` | 直接装配生产 `src/lib.rs`，检查六个公共分解入口的完整调用链、精确覆盖、全域容量与终止、最优性及复用 |
| `axis` | 单位叶子／事件／栅格三条实际输入路径、完整 chord 提取、选线到位图、实际扫描、AoS／packed／sink 与公共包装 |
| `matching` | 真实闭端点扫线、稳定排列与完整 CSR、HKDW 分层／追加增广、终止、最大匹配、Kőnig 证书及不依赖匹配选择的规范选线 |
| `geometry_optimal` | 任意标签像素域的角点、cut 链、竞争分区下界及实际扫描计数上界；含孔洞、分量和角点接触 |
| `index` | 生产索引借用、可变借用范围及整数窄化；违约 abort 的最小映射见源码 |
| `geometry` | 单叶形状核的完整 `u8` 输入域与错误顺序、64／128-bit 掩码的逐位区间语义 |
| `fixed` | 生产固定缓冲的前缀、追加／移除／截断及 Clone／借用／迭代转换 |
| `search` | HK 实际初始化核的长度、初值及未匹配编号完整、有序、无重复；按匹配表稳定过滤有效编号 |
| `chord_graph` | 闭段掩码、CSR 计数与组起点、单组散布、最终标记转换；另含纯数学对偶证书定理 |
| `packed` | 生产输出的构造、清空、原位追加、容量失败不变、编解码与精确借用迭代 |
| `partition` | 真实输出类型上的数学规范、存在性及数量上界等推论；不是分区算法实现证明 |

完整规范、适用域与可信边界见 [矩形模块](../crates/rectangle_decomposition/src/optimal.rs)。
sink 的实际参数／返回事件前缀在驱动中验证，任意闭包内部状态、逐次终止和副作用不在证明域；
panic／进程退出不作为正常错误返回。渐近复杂度和性能另行分析及测量。
普通 Cargo 测试不代替此入口；重复装配的共享模块使各入口验证单元数不能求和。

## 依赖维护

重复依赖的版本集中在根 `Cargo.toml` 的 `workspace.dependencies`，各包保留自身 feature。修改版本约束后运行 `cargo update`，再执行 `features`、上述三条 Cargo 检查和 `docs`。Vulkan 绑定变更另需按设备运行 `compute`／`rt`。

native ash 使用[官方固定提交](https://github.com/ash-rs/ash/commit/8844f6b4eefb16d08cd73c8f3d340e1ac12deb16)，补充正式版尚未提供的 KHR/EXT 接口。wgpu 30.0.1 保留其兼容的 ash 版本，两者通过 Vulkan crate 的原始句柄边界交接。更新时核对上游 API，修改 `crates/vulkan/Cargo.toml` 的 git `rev` 并重新生成 `Cargo.lock`；两者一同审查、提交。

## Git 格式检查

在仓库根目录启用钩子：

```powershell
git config --local core.hooksPath .githooks
```

钩子依赖 Bash、rustfmt 和 clang-format；Git for Windows 已提供 Bash。
Linux/macOS 上确认钩子有执行权限。

提交前仅检查暂存的 `.rs` / `.slang` 及暂存的格式配置，不修改工作树或 index。
Rust 按当前工作区的 edition 2024 检查；Slang 使用 clang-format 的 CSharp 模式。
[Slang 官方编辑器扩展](https://github.com/shader-slang/slang-vscode-extension#clang-format-location-and-style)也采用 clang-format。

在 Git Bash 中手动检查工作树：`bash scripts/check_format.sh`。格式化工作树：

```bash
bash scripts/check_format.sh --fix
```

PowerShell 中的 `bash` 可能指向 WSL，请从 Git Bash 执行以上命令。
部分暂存时，格式化后请自行选择需要暂存的内容。

## GPU 对照测量

`bench_gpu.ps1` 配对执行两个 `shader_bench` 可执行文件，交替 A/B 顺序，保存逐轮 JSON、CSV 及比值汇总，不设性能阈值：

```powershell
powershell -NoProfile -File tests/shaders/bench_gpu.ps1 -Before temp/shader-bench-before.exe -After temp/shader-bench-after.exe
```

两侧使用相同输入和 Slang 版本；脚本关闭 validation，每个进程内部热身。数据含 GPU timestamp 与明确区间的主机时间。负载、失败重放和统计口径见 [GPU 测试](gpu-test.md)。

默认结果写入 `temp/gpu-ab/<unique>`；原始记录与待比较的可执行文件由人工清理。构建缓存和下载的工具留在 `target/`。
