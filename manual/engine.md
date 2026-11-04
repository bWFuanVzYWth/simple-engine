# 引擎接入与功能裁剪

Cargo 用项目根目录的 `Cargo.toml` 记录依赖和功能开关。如果还没有 Rust 项目，可先从引擎根目录运行 `Set-Location ..`，再运行 `cargo new my_engine_app` 创建一个同级目录。打开自己项目里的 `Cargo.toml`，在 `[dependencies]` 下面添加引擎。下面假设你的项目目录和 `simple_engine/` 是同级目录。把 `path` 换成你实际的引擎路径。

```toml
[dependencies]
simple_engine = { path = "../simple_engine", features = ["color", "voxel_mesh"] }
```

若已有 `[dependencies]`，把依赖行放进去即可。用 PowerShell 进入你自己的项目目录并运行 `cargo +nightly-2026-10-31 check`。这里的版本与引擎的 [rust-toolchain.toml](../rust-toolchain.toml) 一致。出现 `Finished` 表示依赖通过编译检查。

若在引擎自己的 workspace（一组共同构建的 Rust 包）里添加项目，可从根目录继承依赖声明：

```toml
[dependencies]
simple_engine = { workspace = true, features = ["color", "voxel_mesh"] }
```

native 渲染和 shader 构建分别声明运行依赖与构建依赖：

```toml
[dependencies]
simple_engine = { workspace = true, features = ["vulkan", "gpu_log"] }

[build-dependencies]
simple_engine = { workspace = true, features = ["shader_build", "shader_library"] }
```

仓库外沿用前面示例的 `path` 写法。Web 选择 `webgpu`；纯图形和图片选择 `render2d_gpu`，文字另选 `render2d_text`；GUI 选择 `ui` 与 `render2d_gpu`，例如 `features = ["webgpu", "ui", "render2d_gpu"]`。完整 feature 清单及依赖关系见[引擎 API](../target/doc/simple_engine/index.html)。

Cargo feature 会叠加。检查裁剪效果时独立检查根包，避免 workspace 其他消费者启用额外功能：

```powershell
cargo check --locked -p simple_engine --no-default-features
cargo check --locked -p simple_engine --no-default-features --features color,voxel_mesh
powershell -NoProfile -File scripts/check.ps1 -Suite features
```

完整构建与平台检查见[开发脚本](development.md)。API 使用示例：

- [输入与录制重放](../target/doc/input/index.html)
- [即时 GUI 的构建与发布](../target/doc/ui/index.html)
- [CPU 2D 场景](../target/doc/render2d/index.html)与[wgpu 编码](../target/doc/render2d/gpu/index.html)
- [体素表面提取](../target/doc/voxel_mesh/index.html)
- [DAG 局部构建](../target/doc/hash_dag/struct.LocalBuilder.html)与[批量导入](../target/doc/hash_dag/struct.HashDag.html#method.set_built_regions)：内核不创建线程，由应用持有结果槽并调度
- [quad 光源 BVH](../target/doc/light_bvh/index.html)与[共享 Slang 采样、RR、MIS](../target/doc/simple_engine/shader_library/index.html)
- [primeDRT 参数准备](../target/doc/simple_engine/display/index.html)与共享 `display.prime_drt` Slang 模块
- [OpenPBR 能力子集](../target/doc/openpbr/index.html)与[运行时大气](../target/doc/atmosphere/index.html)；GPU owner 由调用方显式共享、排序和提交
- [Vulkan 光追与二值 OMM](../target/doc/vulkan/index.html)
- [SipHash](../target/doc/sip_hash/index.html)与[批量梯度噪声](../target/doc/perlin_noise/index.html)
- [GPU 日志](../target/doc/gpu_log/index.html)与[测试设施](../target/doc/gpu_test/index.html)

运行入口见[Cornell box](../examples/cornell_box/README.md)与[hello_triangle](../examples/hello_triangle/README.md)。

显示参数可独立检查：`cargo test --locked -p simple_engine --no-default-features --features display`。GPU 显示算法验证见[生产 shader 测试](gpu-test.md)。更换显示变换时按目标选择线性或编码输出，颜色空间与 transfer 契约见对应 API。
