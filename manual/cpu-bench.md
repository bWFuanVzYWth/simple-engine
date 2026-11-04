# Criterion CPU 基准

使用 Criterion 测量生产内核与应用 CPU 链路，不启动 GPU，也不设置耗时门槛：

```powershell
cargo bench -p rectangle_decomposition --bench optimal
cargo bench -p voxel_mesh --bench sparse
cargo bench -p color --bench conversion
cargo bench -p render2d --bench scene
cargo bench -p render2d --features text --bench text
cargo bench -p hash_dag --bench coverage
cargo bench -p hash_dag --bench diff
cargo bench -p hash_dag --bench built
cargo bench -p sip_hash --bench hash
cargo bench -p perlin_noise --bench noise
cargo bench -p hash_dag --bench dag -- dag/export
cargo bench -p input --bench input
cargo bench -p frame_loop --bench handoff
cargo bench -p chunk_manager --bench planning
cargo bench -p ui --bench ui
```

CPU 基准只需 Rust。按名称过滤以缩小负载，例如：

```powershell
cargo bench -p voxel_mesh --bench sparse -- terrain
```

仓库设置 `CRITERION_HOME` 为工作区 `temp/criterion`，用户环境变量可覆盖。原始样本与基线不可由新一轮运行精确再生，应人工决定清理。保存并比较同环境基线：

```powershell
cargo bench -p voxel_mesh --bench sparse -- --save-baseline before
# 切换到待比较代码后，以同样工具链、参数和系统负载运行：
cargo bench -p voxel_mesh --bench sparse -- --baseline before
```

暖机、采样和输出选项见 [Criterion 命令行说明](https://criterion-rs.github.io/book/user_guide/command_line_options.html)。不要在不同 CPU、worker 数或输入规模间直接比较基线。

选择负载时，先区分复用内核与应用整链：

| 组 | 用途 | 计数单位 |
|---|---|---|
| `rectangle/optimal` / `rectangle/prepare` | 最少矩形划分、输入准备及输出布局 | 输入像素 |
| `mesh/sparse` | 分层树表面提取、准备与析构 | 输入块 |
| `color` | 真实批量色彩空间转换与量化 | 输入颜色 |
| `scene` / `plot` | CPU 场景复用/冷建/校验，分箱与图表几何 | 控件或输入样本 |
| `text` | 字体加载、布局命中/淘汰、栅格命中/淘汰、编辑和命中几何 | 组内声明的字节、glyph 或操作；pair 含两次编辑 |
| `coverage` | DAG 批读写、解码、快照及 CPU 射线 | 每项 `SVDAG_WORK` 声明的元素与调用数 |
| `diff` | 保活根结构差分，冷暖暂存及编辑/快照释放整链 | 组内声明的输入编辑数；共享/noop按调用 |
| `dag_built` | dense 串行导入、复用局部构建、预建结果合并与四线程整链 | 八个64³块；整链包含线程创建、局部与全局去重、合并及析构 |
| `dag/export` | 十层原始数组借用与复用缓冲复制 | 借用按层数；复制按实际字节 |
| `sip_hash` | SipHash-1-3/2-4 字节流、三坐标与 SIMD | 各项声明的消息字节或坐标数 |
| `noise` | 直接噪声、纯梯度、给定梯度插值、批尾部及拒绝路径 | 各项声明的采样数 |
| `input` | 命令批量接收与重放 | 以组内 Criterion throughput 为准 |
| `latest` / `clock` | 完整值交接、会话时钟与绝对 tick 期限 | 以组内声明的载荷和调用为准 |
| `chunk/planning` | 同容量不同半径的首次规划、访问预算变化、稳定目标、单格移动和瞬移；后两者包含即时完成、发布与有界回收 | 每帧一次规划，实际访问与更新数由 Stats 返回 |
| `ui` | 描述、布局、发布或捕获路由 | 以组内 Criterion throughput 为准 |

每个 case 的输入规模与计时边界保留在对应 bench 源码的文档注释：[最少矩形](../crates/rectangle_decomposition/benches/optimal.rs)、[稀疏 mesh](../crates/voxel_mesh/benches/sparse.rs)、[input](../crates/input/benches/input.rs)、[ui](../crates/ui/benches/ui.rs)。

[区块规划](../crates/chunk_manager/benches/planning.rs)排除真实建网与 GPU 工作；首次规划排除状态预分配和输出合并，统一容量为 30_000，并在计时外打印实际访问、请求、替换、阻塞、占槽数及保留字节数。半径 64 只测首帧，不要求容量足以收敛；访问预算组固定半径 4 和输出额度。其余组在计时外预收敛，移动和瞬移每次只推进一帧，不要求追上持续变化的目标。

[局部 DAG 构建](../crates/hashdag/benches/built.rs)分别记录输入、结果槽、builder 与目标池容量。`merge_prebuilt`不含局部构建，`scheduled_4`包含局部和全局两次去重及显式调度；判断并行收益应与`dense_serial`整链比较，不能只使用合并内核耗时。

运行前输出的容量以字节计，反映显式输入、scratch、输出或新旧 Scene 的保留工作集，不是进程驻留集或内存流量。记录提交、编译器、构建参数、CPU、worker 数与系统负载，同时查看内核、整链及工作集结果；不能将统计显著或单内核加速直接解释为应用收益。

文字 bench 使用 [共享 Noto 字体](../assets/fonts/README.md)。`font_new_drop`包含字体复制、解析与析构；其余组预先加载字体。`layout/miss_eviction`轮换预生成正文，`raster/miss_eviction`让键集合超过固定缓存；二者只保证对应层发生 miss，不宣称第三方内部状态完全为冷。编辑 pair 恢复原正文，排版/栅格输出在计时中消费。各负载先以 `-- --test` 检查，再单独采样，避免与构建或 GPU 工作并发。

2D GPU 准备整链另用 `cargo bench -p render2d_tests --bench prepare`，需要 Vulkan compute。它分别测准备、编码提交、完成等待和包含包释放的完整链路，覆盖静态复用、每帧重建、glyph 冷热及多视图在途；计时边界和排除项见 [2D 回归](../tests/render2d/README.md)。不能把完成等待解释为 GPU timestamp。
