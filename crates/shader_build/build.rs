//! 根据 Cargo 的精确输出 profile 固定 shader 调试策略。
#[path = "src/profile.rs"]
mod profile;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/profile.rs");
    println!("cargo::rustc-check-cfg=cfg(shader_release_lto)");
    let out = std::env::var_os("OUT_DIR").expect("Cargo 未设置 OUT_DIR");
    let package = std::env::var("CARGO_PKG_NAME").expect("Cargo 未设置 CARGO_PKG_NAME");
    let name =
        profile::cargo_profile(std::path::Path::new(&out), &package).expect("shader profile 无效");
    if name == "release-lto" {
        println!("cargo::rustc-cfg=shader_release_lto");
    }
}
