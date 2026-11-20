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
                source: "../../shaders/light_bvh/build.slang",
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
    #[cfg(feature = "dlss_rr")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build_streamline()?;
    }
    #[cfg(feature = "nrd")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build_nrd()?;
    }
    Ok(())
}

#[cfg(feature = "nrd")]
fn build_nrd() -> Result<(), Box<dyn std::error::Error>> {
    use std::{path::PathBuf, process::Command};
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let cache = std::path::absolute(cache)?;
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");
    for file in ["nrd/setup.ps1", "nrd/bridge.cpp"] {
        println!("cargo:rerun-if-changed={file}");
    }
    let status = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            "nrd/setup.ps1",
            "-Cache",
        ])
        .arg(&cache)
        .status()?;
    if !status.success() {
        return Err(format!("NRD SDK 构建失败 {status}").into());
    }
    let nrd = cache.join("nrd-src-4.18");
    let nri = cache.join("nri-src-v191");
    let sdk_shaders = nrd.join("Shaders");
    shader_build::build(
        &["pack", "resolve"].map(|entry| shader_build::Recipe {
            source: "../../shaders/denoise/nrd.slang",
            entry,
            stage: "compute",
            output: if entry == "pack" {
                "nrd_pack.spv"
            } else {
                "nrd_resolve.spv"
            },
            target: shader_build::Target::Spirv("spirv_1_4"),
            args: &["-DNRD_COMPILER_DXC=1"],
        }),
        &[
            "../../shaders",
            sdk_shaders.to_str().ok_or("NRD shader 路径不是 UTF-8")?,
        ],
        &[],
    )?;
    cc::Build::new()
        .cpp(true)
        .std("c++17")
        .flag_if_supported("/EHsc")
        .flag_if_supported("/utf-8")
        .include(nrd.join("Include"))
        .include(nrd.join("Integration"))
        .include(nri.join("Include"))
        .file("nrd/bridge.cpp")
        .try_compile("engine_nrd")?;
    for (dir, lib) in [(nrd, "NRD"), (nri, "NRI")] {
        println!(
            "cargo:rustc-link-search=native={}",
            dir.join("_Bin/Release").display()
        );
        println!("cargo:rustc-link-lib=static={lib}");
    }
    for (path, name) in [
        ("nrd-nri-build/Release", "NRI_VK"),
        ("nrd-nri-build/Release", "NRI_Shared"),
        (
            "nrd-build-git/_deps/shadermake-build/Release",
            "ShaderMakeBlob",
        ),
    ] {
        println!(
            "cargo:rustc-link-search=native={}",
            cache.join(path).display()
        );
        println!("cargo:rustc-link-lib=static={name}");
    }
    Ok(())
}

#[cfg(feature = "dlss_rr")]
fn build_streamline() -> Result<(), Box<dyn std::error::Error>> {
    use std::{path::PathBuf, process::Command};

    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() != Ok("x86_64") {
        return Err("锁定的 Streamline SDK 仅支持 Windows x86-64".into());
    }

    println!("cargo:rerun-if-env-changed=STREAMLINE_SDK");
    for path in [
        "streamline/bridge.cpp",
        "streamline/bridge.h",
        "streamline/setup.ps1",
        "streamline/sdk-lock.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    let verification = Command::new("powershell.exe")
        // PowerShell 7 的 PSModulePath 不适用于系统 Windows PowerShell。
        .env_remove("PSModulePath")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); & './streamline/setup.ps1' -VerifyOnly -PassThru",
        ])
        .output()?;
    if !verification.status.success() {
        return Err(format!(
            "Streamline SDK 校验失败：{}{}",
            String::from_utf8_lossy(&verification.stdout),
            String::from_utf8_lossy(&verification.stderr)
        )
        .into());
    }
    let sdk = String::from_utf8(verification.stdout)?;
    let sdk = PathBuf::from(sdk.trim_end_matches(['\r', '\n']));
    if !sdk.is_absolute() {
        return Err("Streamline SDK 校验未返回绝对路径".into());
    }
    println!("cargo:rerun-if-changed={}", sdk.display());
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
