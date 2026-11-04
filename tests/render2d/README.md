# 2D 渲染回归

直接运行 `render2d` 的生产 Slang SPIR-V 与 wgpu renderer，批量检查线性预乘颜色、透明顺序、圆角、边框、裁剪、图片键、文字覆盖率／字体身份、多视图和提交前释放。长 CJK 用例验证约 64KiB 正文的栅格／atlas 工作集随可见区域收敛。

```sh
cargo test -p render2d_tests --test render -- --ignored
cargo test -p render2d_tests --test wgsl -- --ignored
```

`render` 共享一个 Compute 环境。`wgsl` 通过实际 wgpu 验证 render2d 的两个生产 Web 模块。默认 ignored，初始化失败计为失败。颜色属性使用引擎固定的 PropertyRng；当前种子与 stream 写在用例中。

真实链路基准：

```sh
cargo bench -p render2d_tests --bench prepare -- --test
cargo bench -p render2d_tests --bench prepare
```

覆盖静态包复用、每帧重建 2048 个 shape、glyph 冷／热缓存、4 个独立视图与 4 个待完成提交。
`prepare` 包含资源解析、上传排定和独立视图准备；`encode_submit` 包含录制和提交；
`complete` 测提交后的宿主等待／回调，不能当作 GPU timestamp。`total` 直接测全部阶段和包释放。
各阶段独立采样完整链路，不将独立统计分位数相加。每次迭代都完成 GPU 工作，避免 staging 积累。
设备、shader、目标、pipeline 和输入准备不计时；冷 glyph 每次重新准备字体／布局与 renderer，
计时包含 CPU raster 和 atlas miss，不包含字体解析／排版。输出保留资源统计及其排除项；
初始化、提交或完成检查失败即失败。正式采样须避开其他构建与 GPU 工作。
