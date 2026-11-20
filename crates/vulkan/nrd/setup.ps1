# 拉取上游锁定修订并只编译 Vulkan 后端与 SPIR-V。缓存均可删除重建。
param([Parameter(Mandatory=$true)][string]$Cache)
$ErrorActionPreference = 'Stop'
function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program 失败 ($LASTEXITCODE)" }
}
function Get-Source([string]$Folder, [string]$Remote, [string]$Revision) {
    $dest = Join-Path $Cache $Folder
    if (!(Test-Path (Join-Path $dest '.git'))) { Invoke-Checked git @('init', $dest) }
    $actual = & git -C $dest rev-parse --verify --quiet HEAD
    if ($actual -ne $Revision) {
        Invoke-Checked git @('-C', $dest, 'fetch', '--depth=1', $Remote, $Revision)
        Invoke-Checked git @('-C', $dest, 'checkout', '--detach', $Revision)
    }
}
function Build-Sdk {
    Get-Source nrd-src-4.18 https://github.com/NVIDIA-RTX/NRD.git 8f12a22fc90e6a3c7f5da4c9c3103633c842963a
    Get-Source nri-src-v191 https://github.com/NVIDIA-RTX/NRI.git 46c93d79f97c6def986d1ad655165482fbd566a7
    Get-Source nrd-shadermake https://github.com/NVIDIA-RTX/ShaderMake.git 3f01623a76b092ac127d2debd79a672f55d77665
    Get-Source nrd-math-v11 https://github.com/NVIDIA-RTX/MathLib.git 89396dd8a3c9ead59e7f1f85c5396594bd22b8a7
    Get-Source nrd-vk-headers https://github.com/KhronosGroup/Vulkan-Headers.git 9fbc2a782ee3f8a4db64e7d991e23da75e893f44
    Get-Source nrd-vma https://github.com/GPUOpen-LibrariesAndSDKs/VulkanMemoryAllocator.git 0f1a757da579831283413295cc7f3b47eb521c77
    Invoke-Checked cmake @('-S', "$Cache/nrd-src-4.18", '-B', "$Cache/nrd-build-git", '-A','x64',
        '-DNRD_STATIC_LIBRARY=ON','-DNRD_NRI=OFF','-DNRD_EMBEDS_SPIRV_SHADERS=ON',
        '-DNRD_EMBEDS_DXIL_SHADERS=OFF','-DNRD_EMBEDS_DXBC_SHADERS=OFF','-DNRD_SUPPORTS_QUAD_INTRINSICS=OFF',
        '-DSHADERMAKE_FIND_FXC=OFF','-DSHADERMAKE_FIND_DXC=OFF',"-DSHADERMAKE_DXC_VK_PATH=$env:VULKAN_SDK/Bin/dxc.exe",
        "-DFETCHCONTENT_SOURCE_DIR_SHADERMAKE=$Cache/nrd-shadermake","-DFETCHCONTENT_SOURCE_DIR_MATHLIB=$Cache/nrd-math-v11")
    Invoke-Checked cmake @('--build', "$Cache/nrd-build-git", '--config', 'Release', '--target','NRD','--parallel')
    Invoke-Checked cmake @('-S', "$Cache/nri-src-v191", '-B', "$Cache/nrd-nri-build", '-A','x64',
        "-DCMAKE_CXX_FLAGS_RELEASE=/O2 /Ob2 /DNDEBUG /experimental:deterministic /pathmap:`"$Cache\nri-src-v191\Source=/build/nri/Source`"",
        '-DNRI_STATIC_LIBRARY=ON','-DNRI_ENABLE_VK_SUPPORT=ON','-DNRI_ENABLE_D3D11_SUPPORT=OFF',
        '-DNRI_ENABLE_D3D12_SUPPORT=OFF','-DNRI_ENABLE_NONE_SUPPORT=OFF','-DNRI_ENABLE_WGPU_SUPPORT=OFF',
        '-DNRI_ENABLE_NVTX_SUPPORT=OFF','-DNRI_ENABLE_VALIDATION_SUPPORT=OFF',
        "-DFETCHCONTENT_SOURCE_DIR_VULKAN_HEADERS=$Cache/nrd-vk-headers","-DFETCHCONTENT_SOURCE_DIR_VMA=$Cache/nrd-vma")
    Invoke-Checked cmake @('--build', "$Cache/nrd-nri-build", '--config','Release','--target','NRI','--parallel')
}

$Cache = [IO.Path]::GetFullPath($Cache)
if (!$env:VULKAN_SDK -or !(Test-Path -LiteralPath (Join-Path $env:VULKAN_SDK 'Bin/dxc.exe'))) {
    throw '构建 NRD 需要将 VULKAN_SDK 指向含有 Bin/dxc.exe 的 Vulkan SDK。'
}
[void][IO.Directory]::CreateDirectory($Cache)
# 引擎和嵌套应用可以使用不同 Cargo target，同时访问这一份 SDK 构建缓存。
$key = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
    [Text.Encoding]::UTF8.GetBytes($Cache.ToUpperInvariant())))
$mutex = [Threading.Mutex]::new($false, "Local\simple-engine-nrd-$key")
$locked = $false
try {
    try { $locked = $mutex.WaitOne() } catch [Threading.AbandonedMutexException] { $locked = $true }
    Build-Sdk
} finally {
    if ($locked) { $mutex.ReleaseMutex() }
    $mutex.Dispose()
}
