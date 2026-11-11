//! 构建原生地形准备内核及可选的 Windows Streamline bridge。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    shader_build::build(
        &[
            shader_build::Recipe {
                source: "shaders/expand_quads.slang",
                entry: "expand_quads",
                stage: "compute",
                output: "expand_quads.spv",
                target: shader_build::Target::Spirv("spirv_1_4"),
                args: &["-fp-mode", "precise"],
            },
            shader_build::Recipe {
                source: "../../shaders/pt/light_build.slang",
                entry: "build_level",
                stage: "compute",
                output: "light_build.spv",
                target: shader_build::Target::Spirv("spirv_1_4"),
                args: &["-fp-mode", "precise"],
            },
        ],
        &["../../shaders"],
        &[],
    )?;
    println!("cargo:rerun-if-env-changed=STREAMLINE_SDK");
    #[cfg(feature = "dlss_rr")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build_streamline()?;
    }
    Ok(())
}

#[cfg(feature = "dlss_rr")]
fn build_streamline() -> Result<(), Box<dyn std::error::Error>> {
    use std::{path::PathBuf, process::Command};

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64") {
        return Err("锁定的 Streamline SDK 仅支持 Windows x86-64".into());
    }

    let sdk = PathBuf::from(std::env::var_os("STREAMLINE_SDK").ok_or(
        "dlss_rr 需要显式 STREAMLINE_SDK；运行 scripts/install_streamline.ps1 安装锁定 SDK",
    )?);
    if !sdk.is_absolute() {
        return Err("STREAMLINE_SDK 必须为绝对路径".into());
    }
    for path in [
        "streamline/bridge.cpp",
        "streamline/bridge.h",
        "streamline/setup.ps1",
        "streamline/sdk-lock.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed={}", sdk.display());
    let verification = Command::new("powershell.exe")
        // PowerShell 7 的 PSModulePath 不适用于系统 Windows PowerShell。
        .env_remove("PSModulePath")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "streamline/setup.ps1",
            "-SdkRoot",
        ])
        .arg(&sdk)
        .arg("-VerifyOnly")
        .output()?;
    if !verification.status.success() {
        return Err(format!(
            "Streamline SDK 校验失败：{}{}",
            String::from_utf8_lossy(&verification.stdout),
            String::from_utf8_lossy(&verification.stderr)
        )
        .into());
    }
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/utf-8")
        .include(sdk.join("include"))
        .include(sdk.join("vulkan-headers/include"))
        .file("streamline/bridge.cpp")
        .try_compile("simple_engine_streamline")?;
    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("缺少 OUT_DIR")?);
    // 自定义profile的PROFILE仍可能为release；从Cargo的build包输出槽定位真实目录。
    // 同时适用于build/<package-hash>/out及build/<package>/<hash>/out。
    let profile = output
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "build"))
        .and_then(std::path::Path::parent)
        .ok_or("OUT_DIR 不在 Cargo profile 目录内")?;
    for target in [profile.to_path_buf(), profile.join("deps"), output.clone()] {
        std::fs::create_dir_all(&target)?;
        for name in [
            "sl.interposer.dll",
            "sl.common.dll",
            "sl.dlss_d.dll",
            "nvngx_dlssd.dll",
        ] {
            std::fs::copy(sdk.join("bin/x64").join(name), target.join(name))?;
        }
        for (source, name) in [
            ("licenses/NVIDIA-Streamline.txt", "NVIDIA-Streamline.txt"),
            ("3rd-party-licenses.md", "streamline-licenses.md"),
            ("bin/x64/nvngx_dlss.license.txt", "nvngx_dlss.license.txt"),
        ] {
            std::fs::copy(sdk.join(source), target.join(name))?;
        }
    }
    Ok(())
}
