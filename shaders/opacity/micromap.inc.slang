// KHR OMM 的 RayQuery 逐入口准入；TraceRay 仍由宿主管线 flag 控制。
// Slang 原生 SPIR-V 汇编，不修改编译产物；启用此模式的设备须支持 VK_KHR_opacity_micromap。
// https://docs.vulkan.org/spec/latest/chapters/raytraversal.html#ray-opacity-micromap
// clang-format off: SPIR-V 的 %id 与 $Entry 保持原样。
#ifndef ENGINE_OPACITY_MICROMAP
#define ENGINE_OPACITY_MICROMAP(Entry)                  \
    spirv_asm                                          \
    {                                                  \
        OpCapability RayTracingOpacityMicromapExecutionModeKHR; \
        OpExtension "SPV_KHR_opacity_micromap";          \
        %enabled = OpSpecConstantTrue $$bool;            \
        OpExecutionModeId $Entry OpacityMicromapIdKHR %enabled; \
    };
#endif
// clang-format on
