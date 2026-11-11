param(
    [ValidateSet('cpu', 'features', 'build', 'docs', 'compute', 'rt')][string]$Suite = 'cpu',
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 900
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding $false
. (Join-Path $PSScriptRoot 'process.ps1')
$taskRoot = Split-Path -Parent $PSScriptRoot
$run = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$logs = Join-Path $taskRoot "temp/check/$Suite/$run"
$step = 0

function Invoke-Step([string]$Name, [string]$Program, [string[]]$Arguments) {
    $script:step++
    Write-Host "[$Suite/$script:step] $Name"
    $prefix = Join-Path $logs "$script:step-$Name"
    $null = Invoke-CheckedProcess -FilePath $Program -ArgumentList $Arguments -WorkingDirectory $taskRoot -TimeoutSeconds $TimeoutSeconds -LogPrefix $prefix
}

function Invoke-Cargo([string]$Name, [string[]]$Arguments) {
    Invoke-Step $Name (Get-Command cargo -ErrorAction Stop).Source $Arguments
}

function Find-Bash {
    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        $git = (Get-Command git -ErrorAction Stop).Source
        $candidate = Join-Path (Split-Path -Parent (Split-Path -Parent $git)) 'bin/bash.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
        $candidate = Join-Path $env:ProgramFiles 'Git/bin/bash.exe'
        if (Test-Path -LiteralPath $candidate) { return $candidate }
        throw '格式检查需要 Git Bash。'
    }
    return (Get-Command bash -ErrorAction Stop).Source
}

function Read-Coverage([string]$Directory, $Targets) {
    foreach ($target in $Targets) {
        $report = Join-Path $Directory "$($target.name.Replace('-', '_')).json"
        $entries = (Get-Content -LiteralPath $report -Raw | ConvertFrom-Json).PSObject.Properties
        $total = 0
        $documented = 0
        foreach ($entry in $entries) {
            $total += $entry.Value.total
            $documented += $entry.Value.with_docs
        }
        if ($total -eq 0 -or $documented -ne $total) { throw "公开文档覆盖不足：$($target.name) $documented/$total" }
        [pscustomobject]@{ crate = $target.name; documented = $documented; total = $total }
    }
}

try {
    Push-Location $taskRoot
    switch ($Suite) {
        'cpu' {
            Invoke-Step 'format' (Find-Bash) @('scripts/check_format.sh')
            $packages = @('-p', 'camera', '-p', 'color', '-p', 'input', '-p', 'frame_loop', '-p', 'ui', '-p', 'render2d', '-p', 'hash_dag', '-p', 'sip_hash', '-p', 'perlin_noise', '-p', 'rectangle_decomposition', '-p', 'voxel_mesh', '-p', 'chunk_manager', '-p', 'light_bvh', '-p', 'light_sampling', '-p', 'opacity', '-p', 'shader_build', '-p', 'gpu_log', '-p', 'gpu_test')
            Invoke-Cargo 'test' (@('test', '--locked', '--lib') + $packages)
            Invoke-Cargo 'clippy' (@('clippy', '--locked', '--all-targets') + $packages)
            Invoke-Cargo 'chunk-manager-lifecycle' @('test', '--locked', '-p', 'chunk_manager', '--tests')
            Invoke-Cargo 'restir-pt-cpu' @('test', '--locked', '-p', 'restir_pt', '--tests')
            Invoke-Cargo 'input-recording' @('test', '--locked', '-p', 'input', '--features', 'serde')
            Invoke-Cargo 'display-cpu' @('test', '--locked', '-p', 'simple_engine', '--no-default-features', '--features', 'display', '--lib')
            Invoke-Cargo 'render-algorithms-cpu' @('test', '--locked', '-p', 'openpbr', '-p', 'atmosphere', '-p', 'pt', '--no-default-features', '--lib')
            Invoke-Step 'process-tests' (Get-Process -Id $PID).Path @('-NoProfile', '-File', (Join-Path $PSScriptRoot 'tests/process.ps1'))
        }
        'features' {
            $cases = @(
                @{ Name = 'minimal' },
                @{ Name = 'camera'; Features = 'camera' },
                @{ Name = 'display'; Features = 'display' },
                @{ Name = 'openpbr'; Features = 'openpbr' },
                @{ Name = 'openpbr-program'; Features = 'openpbr_program' },
                @{ Name = 'openpbr-gpu'; Features = 'openpbr_gpu' },
                @{ Name = 'pt'; Features = 'pt' },
                @{ Name = 'pt-gpu'; Features = 'pt_gpu' },
                @{ Name = 'restir-pt'; Features = 'restir_pt' },
                @{ Name = 'restir-pt-gpu'; Features = 'restir_pt_gpu' },
                @{ Name = 'atmosphere'; Features = 'atmosphere' },
                @{ Name = 'atmosphere-gpu'; Features = 'atmosphere_gpu' },
                @{ Name = 'atmosphere-native'; Features = 'atmosphere_native' },
                @{ Name = 'cpu'; Features = 'color,voxel_mesh,hash_dag,light_bvh' },
                @{ Name = 'chunk-manager'; Features = 'chunk_manager' },
                @{ Name = 'scene-cache'; Features = 'scene_cache' },
                @{ Name = 'light-sampling'; Features = 'light_sampling' },
                @{ Name = 'opacity'; Features = 'opacity' },
                @{ Name = 'sip-hash'; Features = 'sip_hash' },
                @{ Name = 'perlin-noise'; Features = 'perlin_noise' },
                @{ Name = 'input'; Features = 'input' },
                @{ Name = 'input-recording'; Features = 'input_recording' },
                @{ Name = 'frame-loop'; Features = 'frame_loop' },
                @{ Name = 'render2d'; Features = 'render2d' },
                @{ Name = 'render2d-text'; Features = 'render2d_text' },
                @{ Name = 'render2d-gpu'; Features = 'render2d_gpu' },
                @{ Name = 'render2d-gpu-text'; Features = 'render2d_gpu,render2d_text' },
                @{ Name = 'ui'; Features = 'ui' },
                @{ Name = 'gpu-log'; Features = 'gpu_log' },
                @{ Name = 'shader-build'; Features = 'shader_build' },
                @{ Name = 'shader-library'; Features = 'shader_library' },
                @{ Name = 'wgpu'; Features = 'wgpu' },
                @{ Name = 'native'; Features = 'vulkan,gpu_log' },
                @{ Name = 'gpu-test'; Features = 'gpu_test' },
                @{ Name = 'all'; All = $true },
                @{ Name = 'web'; Features = 'webgpu,render2d_gpu'; Target = 'wasm32-unknown-unknown' },
                @{ Name = 'web-cpu'; Features = 'color,display,openpbr,pt,restir_pt,atmosphere,voxel_mesh,chunk_manager,light_bvh,perlin_noise,input_recording,ui'; Target = 'wasm32-unknown-unknown' }
            )
            foreach ($case in $cases) {
                $arguments = @('check', '--locked', '-p', 'simple_engine', '--no-default-features')
                if ($case.Features) { $arguments += @('--features', $case.Features) }
                if ($case.All) { $arguments += '--all-features' }
                if ($case.Target) { $arguments += @('--target', $case.Target) }
                Invoke-Cargo $case.Name $arguments
            }
            foreach ($package in @('hello_triangle', 'cornell_box')) {
                Invoke-Cargo "$package-system" @('check', '--locked', '-p', $package, '--no-default-features', '--all-targets')
            }
        }
        'build' {
            Invoke-Cargo 'workspace-test' @('test', '--locked', '--workspace')
            Invoke-Cargo 'clippy' @('clippy', '--locked', '--workspace', '--all-targets')
            Invoke-Cargo 'wasm' @('check', '--locked', '-p', 'simple_engine', '--features', 'webgpu,render2d_gpu', '--target', 'wasm32-unknown-unknown')
        }
        'docs' {
            $previousDocFlags = $env:RUSTDOCFLAGS
            $docFlags = '-F missing_docs -F rustdoc::broken_intra_doc_links -F rustdoc::private_intra_doc_links -F rustdoc::invalid_html_tags'
            try {
                $env:RUSTDOCFLAGS = "$previousDocFlags $docFlags".Trim()
                $metadata = (& cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json)
                if ($LASTEXITCODE -ne 0) { throw '无法读取工作区文档目标' }
                $targets = @($metadata.packages | ForEach-Object { $_.targets } | Where-Object { $_.kind -contains 'lib' -or $_.kind -contains 'rlib' })
                $webPackages = @('simple_engine', 'camera', 'color', 'gpu_log', 'hash_dag', 'sip_hash', 'perlin_noise', 'input', 'rectangle_decomposition', 'voxel_mesh', 'chunk_manager', 'light_bvh', 'light_sampling', 'opacity', 'render2d', 'ui')
                $webArgs = @('doc', '--locked', '--target', 'wasm32-unknown-unknown', '--no-deps', '--features', 'simple_engine/camera,simple_engine/color,simple_engine/display,simple_engine/gpu_log,simple_engine/hash_dag,simple_engine/perlin_noise,simple_engine/input_recording,simple_engine/voxel_mesh,simple_engine/chunk_manager,simple_engine/light_bvh,simple_engine/light_sampling,simple_engine/opacity,simple_engine/shader_library')
                foreach ($package in $webPackages) { $webArgs += @('-p', $package) }
                $env:RUSTDOCFLAGS += ' -Z unstable-options --show-coverage --output-format json'
                Invoke-Cargo 'coverage' @('doc', '--locked', '--workspace', '--lib', '--all-features', '--no-deps')
                $native = @(Read-Coverage (Join-Path $metadata.target_directory 'doc') $targets)
                Invoke-Cargo 'web-coverage' $webArgs
                $web = @(Read-Coverage (Join-Path $metadata.target_directory 'wasm32-unknown-unknown/doc') ($targets | Where-Object { $webPackages -contains $_.name }))
                $coverage = Join-Path $metadata.target_directory 'doc-coverage.json'
                [pscustomobject]@{ native = $native; web = $web } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $coverage -Encoding UTF8
                # coverage 与 HTML 共用 rustdoc 输出目录，最后生成可阅读的页面。
                $env:RUSTDOCFLAGS = "$previousDocFlags $docFlags".Trim()
                Invoke-Cargo 'html' @('doc', '--locked', '--workspace', '--all-features', '--no-deps')
                Invoke-Cargo 'examples' @('test', '--locked', '--workspace', '--all-features', '--doc')
                Invoke-Cargo 'web' $webArgs
                Write-Host "公开文档覆盖 100%；报告：$coverage"
            } finally {
                $env:RUSTDOCFLAGS = $previousDocFlags
            }
        }
        'compute' {
            Invoke-Cargo 'compile' @('test', '--locked', '-p', 'shader_tests', '--no-run')
            $previousDebug = $env:WGPU_DEBUG
            $previousValidation = $env:WGPU_VALIDATION
            $previousSync = $env:VK_LAYER_VALIDATE_SYNC
            try {
                $env:WGPU_DEBUG = '1'
                $env:WGPU_VALIDATION = '1'
                $env:VK_LAYER_VALIDATE_SYNC = '1'
                Invoke-Cargo 'compute' @('test', '--locked', '-p', 'shader_tests', '--test', 'shaders', '--', '--ignored', '[compute]')
                Invoke-Cargo 'pt-foundations' @('test', '--locked', '-p', 'shader_tests', '--test', 'light_bvh', '--test', 'sampling', '--test', 'binary_sobol', '--test', 'pt_scene', '--test', 'denoise', '--test', 'reconstruction', '--', '--include-ignored', '--test-threads=1')
                Invoke-Cargo 'restir-pt' @('test', '--locked', '-p', 'shader_tests', '--test', 'restir_pt', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'display' @('test', '--locked', '-p', 'shader_tests', '--test', 'display', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'exposure' @('test', '--locked', '-p', 'shader_tests', '--test', 'exposure', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'openpbr' @('test', '--locked', '-p', 'openpbr_tests', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'openpbr-artifacts' @('test', '--locked', '-p', 'openpbr_tests', '--test', 'specialization')
                Invoke-Cargo 'atmosphere' @('test', '--locked', '-p', 'atmosphere', '--lib', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'atmosphere-native' @('test', '--locked', '-p', 'atmosphere', '--all-features', '--lib', 'native::', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'atmosphere-math' @('test', '--locked', '-p', 'shader_tests', '--test', 'atmosphere', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'replay' @('test', '--locked', '-p', 'shader_tests', '--test', 'replay', '--', '--ignored')
                Invoke-Cargo 'render2d' @('test', '--locked', '-p', 'render2d_tests', '--test', 'render', '--', '--ignored')
                Invoke-Cargo 'web-wgsl' @('test', '--locked', '-p', 'render2d_tests', '--test', 'wgsl', '--', '--ignored', '--test-threads=1')
            } finally {
                $env:WGPU_DEBUG = $previousDebug
                $env:WGPU_VALIDATION = $previousValidation
                $env:VK_LAYER_VALIDATE_SYNC = $previousSync
            }
        }
        'rt' {
            Invoke-Cargo 'compile' @('test', '--locked', '-p', 'shader_tests', '-p', 'cornell_box', '-p', 'wgpu_vulkan_tests', '--no-run')
            $previousDebug = $env:WGPU_DEBUG
            $previousValidation = $env:WGPU_VALIDATION
            $previousSync = $env:VK_LAYER_VALIDATE_SYNC
            try {
                $env:WGPU_DEBUG = '1'
                $env:WGPU_VALIDATION = '1'
                $env:VK_LAYER_VALIDATE_SYNC = '1'
                Invoke-Cargo 'rt' @('test', '--locked', '-p', 'shader_tests', '--test', 'shaders', '--', '--ignored', '[rt]')
                Invoke-Cargo 'timing' @('test', '--locked', '-p', 'shader_tests', '--test', 'timing', '--', '--ignored')
                Invoke-Cargo 'examples' @('test', '--locked', '-p', 'cornell_box', '--', '--ignored')
                Invoke-Cargo 'interop' @('test', '--locked', '-p', 'wgpu_vulkan_tests', '--test', 'interop', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'instances' @('test', '--locked', '-p', 'wgpu_vulkan_tests', '--test', 'instances', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'sampled' @('test', '--locked', '-p', 'wgpu_vulkan_tests', '--test', 'sampled', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'callable' @('test', '--locked', '-p', 'wgpu_vulkan_tests', '--test', 'callable', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'buffers' @('test', '--locked', '-p', 'wgpu_vulkan_tests', '--test', 'buffers', '--', '--ignored', '--test-threads=1')
                Invoke-Cargo 'native' @('test', '--locked', '-p', 'wgpu_vulkan_tests', '--test', 'native', '--', '--ignored', '--test-threads=1')
            } finally {
                $env:WGPU_DEBUG = $previousDebug
                $env:WGPU_VALIDATION = $previousValidation
                $env:VK_LAYER_VALIDATE_SYNC = $previousSync
            }
        }
    }
    Write-Host "完成 $Suite；其它档未执行。日志：$logs"
} catch {
    Write-Error -ErrorRecord $_ -ErrorAction Continue
    $code = $_.Exception.Data['ExitCode']
    if ($null -eq $code -or $code -eq 0) { $code = 1 }
    exit $code
} finally {
    Pop-Location
}
