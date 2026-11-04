# OpenPBR 支持子域验证

```powershell
cargo test --locked -p openpbr
cargo test --locked -p openpbr_tests --test specialization
cargo test --locked -p openpbr_tests --test gpu --no-run
# 需要真实 Vulkan GPU；与其他 GPU 验证／性能运行互斥。
cargo test --locked -p openpbr_tests --test gpu -- --ignored --test-threads=1
```

GPU fixture 使用生产 `GpuEnergy` 的原始 HALF4 3D texture、linear clamp sampler 和直接 Slang SPIR-V，
不引入软件替代、Naga 或 shader 插桩。验证上传轴序／原始系数、完整连续 sample/eval 边缘 PDF、
法向 Fresnel／TIR／介质转换以及极端有限 PDF 的稳定贡献计算。

随包保留的第三方原文见 [Apache-2.0](LICENSES/robocute-Apache-2.0.txt) 与[来源声明](LICENSES/robocute-NOTICE.txt)。

GPU tests 默认 ignored；`--no-run` 和 CPU 参数／asset 检查不代表 GPU 运行通过。

`specialized-*.spv` 分别以八个编译期类别生成，独立 GPU 用例比较特化闭包与通用入口，
并比较同命中 `shade_specialized` 的 mask／continuation 结果与独立 evaluate/sample。
conductor、thin 及三个 delta fixture 不创建或绑定能量纹理／sampler；CPU
`compiled_profiles_keep_only_required_energy_resources` 直接检查最终 SPIR-V 的可达调用图、
资源加载与图像操作，不以嵌入的 debug 源码字符串判定特化。
独立对照另保留不量化的 SSS 权重与任意有效薄片厚度；八个类别不替代通用 fractional-metal 入口。

连续闭包测试独立产生 sample/evaluate，并按 `3e-5` 比较 response/pdf。窄 GGX 的原始密度比较单独计入 half-vector 重建的 f32
舍入条件数：`|d log(D)/dθ|≤2(1−alpha²)/alpha`，`alpha=roughness²`，thin 另乘原算法的缩放。
该 fixture 的保守舍入模型不代表所有 GPU 的严格精度保证，也不扩大生产算法的输入支持域。

组合版本的参数证明与绑定契约见 `openpbr::{MaterialRanges,InterfaceIor,FlowKey,FlowPlan}`，
shader 入口见 `bsdf.openpbr.flow`。`flow-*.spv` 覆盖同一面上金属／非金属、diffuse／SSS、
delta／continuous 的组合，以及匹配 IOR、黑色 base 和无散射的边界；并与原通用入口逐样本比较。
可达指令检查包括未声明物理族、资源读取和常量条件分支；保留 Fresnel、TIR、随机选择等数据条件。

修改前先单独保存旧 SPIR-V，修改后使用同一进程交替测量两条真实 pipeline：

先按[本机配置](../../manual/local.md)设置 `ENGINE_OPENPBR_BEFORE` / `ENGINE_OPENPBR_AFTER` 并加载当前会话。

```powershell
cargo bench --locked -p openpbr_tests --bench flow -- --before $env:ENGINE_OPENPBR_BEFORE --after $env:ENGINE_OPENPBR_AFTER --profile opaque --output temp/flow/opaque.json
```

`flow` 默认 65,536 条相同输入、64 dispatch、21 对交替顺序，记录 GPU timestamp、宿主等待与录制成本。
输入生成、编译、pipeline 创建及数值核验不计时；产物、输入 hash 和全部样本写入 JSON。
`profile` 可选 opaque、opaque-smooth、subsurface、sss-only、conductor、solid、thin、
delta-conductor、delta-solid、delta-thin、mixed、mixed-sss、mixed-smooth。
必须选择覆盖该输入域的两个产物；整体 PT 需要另用固定场景与样本进行配对测量。
