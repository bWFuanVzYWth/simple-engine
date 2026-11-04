//! 独立桌面进程中的 Surface resize 与呈现回归。
#![forbid(unsafe_code)]

fn main() -> Result<(), wgpu_vulkan_tests::Error> {
    wgpu_vulkan_tests::window_resize_probe()
}
