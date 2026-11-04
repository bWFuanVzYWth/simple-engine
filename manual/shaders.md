# 共享 Slang 模块接入

在构建依赖启用 `shader_build` 与 `shader_library`，由源码入口提供 include 目录：

```rust
use simple_engine::{shader_build, shader_library};

shader_build::build(&recipes, &[shader_library::root()], &[])?;
```

recipe 的完整写法见[构建 API 示例](../target/doc/shader_build/index.html#buildrs-示例)。源码与许可证随引擎包分发，无需工作区相对路径。

按需要导入模块：

```slang
import pt.sampling;
import pt.bsdf;
import pt.mis;
import geometry.ray;
```

主文件使用 `module`，跨模块接口使用 `public`。新增或拆分模块时参考 [Slang 模块与访问控制](https://docs.shader-slang.org/en/latest/external/slang/docs/user-guide/04-modules-and-access-control.html)。

[模块 API 索引](../target/doc/simple_engine/shader_library/index.html)链接来源与硬件边界；参数、数值前提及返回语义保留在对应 [Slang 声明](../shaders)。不要从手册复制误差系数、矩阵或内存布局。修改后运行[生产 shader 测试](gpu-test.md)，按实际涉及的 compute/RT 路径选择设备；移植几何算法时同时核对 [NVIDIA 许可证](../LICENSES/NVIDIA-self-intersection-avoidance.txt)。
