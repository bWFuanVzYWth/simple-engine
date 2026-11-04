# 快速开始

可以先用 CPU 检查确认源码能编译，也可以直接准备显卡环境，运行一个窗口示例。先把引擎源码放在本机。若手上是源码压缩包，先解压。下面的命令按顺序执行。

## 打开终端并确认位置

1. 安装 rustup。它会根据仓库的 [rust-toolchain.toml](../rust-toolchain.toml) 选择所需版本的 Rust，并提供构建工具 Cargo。Windows 默认使用 MSVC 版 Rust，请同时准备 x64 C++ 构建工具和 Windows SDK。可从官方页面安装。[Rust 安装](https://rust-lang.org/tools/install/)、[微软 C++ 构建工具](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
2. 打开 **PowerShell 7**。把下面命令里的路径替换为你存放引擎源码的目录，再回车。引擎根目录是含有 `Cargo.toml` 和 `rust-toolchain.toml` 的那一层。[PowerShell 7 安装](https://learn.microsoft.com/en-us/powershell/scripting/install/install-powershell-on-windows)

```powershell
Set-Location 'D:/projects/simple_engine'
Get-Location
Test-Path ./Cargo.toml
Test-Path ./rust-toolchain.toml
```

两条 `Test-Path` 都应显示 `True`。若显示 `False`，先换到正确目录，再执行后续命令。PowerShell 的 `./` 指当前目录，路径有空格时保留引号。

## 第一次构建

以下检查只选用颜色和体素建网组件，适合先确认 Rust 环境。首次运行需要下载固定工具链或依赖，之后可复用缓存。

```powershell
rustup --version
cargo --version
cargo check --locked -p simple_engine --no-default-features --features color,voxel_mesh
```

前两条应显示版本，最后一条只检查编译，成功时会出现 `Finished` 并回到命令行。找不到 `cargo` 时先检查 rustup 安装并重新打开终端。若 Windows 提示找不到 `link.exe`，请补齐 MSVC x64 C++ 构建工具和 Windows SDK。

## 打开三角形窗口

先安装显卡驱动，确认它提供 Vulkan 1.4 loader 和支持 Vulkan 的设备。loader 是程序连接 Vulkan 驱动的入口。Windows 上默认面向 `x86-64-v3` 处理器，较老处理器可按 [原生设备手册](native.md) 改用较低目标。

Slang 把着色器（运行在 GPU 上的程序）编译为显卡能使用的格式。在刚才的 PowerShell 中执行下面两条。安装脚本将编译器路径写入当前终端的环境，换终端后需重新设置。

```powershell
./scripts/install_slang.ps1
cargo run --locked -p hello_triangle
```

安装脚本成功时会显示 Slang 的版本和 `SLANGC` 路径。随后应打开显示三角形的窗口，关闭窗口即可退出。构建提示找不到 Slang 时，回到执行安装脚本的终端，或查看 [Slang 排障](shader-build.md)。窗口启动时报 Vulkan 或适配器错误时，核对显卡驱动和[设备要求](native.md)。

## 生成一张光追图片

Cornell Box 是一个小场景，用于观察光照在室内表面之间的反弹。它需要支持 Vulkan 硬件光追及 `VK_KHR_device_address_commands` 的设备。环境准备同上，然后运行：

```powershell
cargo run --locked --release -p cornell_box -- --size 128 --samples 64
```

成功后可在 `target/cornell_box/` 打开 `reference.png`。同目录还有用于比较的 EXR 高动态范围图像和 JSON 运行记录。参数与故障说明见 [示例页面](../examples/cornell_box/README.md)。

## 继续使用

- 想把组件放进自己的 Rust 项目，读[引擎接入](engine.md)。
- 想构建面向浏览器的库，先运行 `rustup target add wasm32-unknown-unknown`，再执行 `cargo check --locked -p simple_engine --features webgpu,render2d_gpu --target wasm32-unknown-unknown`。成功表示编译检查通过，浏览器运行还需要应用提供网页宿主。
- 想读接口或验证代码，见[生成文档](README.md#生成文档)和[开发检查](development.md)。
