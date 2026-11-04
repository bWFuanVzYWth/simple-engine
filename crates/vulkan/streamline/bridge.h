#pragma once
#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C"
{
#endif

    // ABI 3。进程只允许一个 renderer；所有接口串行调用。
    // configure/destroy/shutdown 前，宿主必须证明此前 GPU evaluate 已完成。
    // 字符串在调用期间有效，bridge 在初始化前复制。身份由宿主提供。
    typedef struct SeSlApplication
    {
        const char *engine_version, *project_id;
    } SeSlApplication;

    typedef struct SeSlInit
    {
        uint64_t instance, physical_device, device;
        uint32_t queue_family, queue_index;
    } SeSlInit;

    typedef struct SeSlSize
    {
        uint32_t width, height;
    } SeSlSize;

    enum
    {
        SE_SL_DLAA,
        SE_SL_QUALITY,
        SE_SL_BALANCED,
        SE_SL_PERFORMANCE,
        SE_SL_ULTRA_PERFORMANCE
    };

    typedef struct SeSlImage
    {
        uint64_t image, view, memory;
        uint32_t width, height, format, layout, usage, reserved;
    } SeSlImage;

    enum
    {
        SE_SL_NOISY,
        SE_SL_DEPTH,
        SE_SL_MOTION,
        SE_SL_NORMAL_ROUGHNESS,
        SE_SL_DIFFUSE_ALBEDO,
        SE_SL_SPECULAR_ALBEDO,
        SE_SL_OUTPUT,
        SE_SL_SPECULAR_HIT_DISTANCE,
        SE_SL_SPECULAR_MOTION,
        SE_SL_IMAGE_COUNT
    };

    typedef struct SeSlFrame
    {
        uint64_t command;
        uint32_t index, reset;
        float jitter[2], near_plane, far_plane, fov, aspect;
        float position[3], up[3], right[3], forward[3];
        // 行主序、行向量、无 jitter。motion 为左上原点、输入像素 previous-current。
        float world_to_view[16], view_to_world[16], view_to_clip[16], clip_to_view[16];
        float clip_to_previous[16], previous_to_clip[16];
        // 单 mip/layer、GENERAL，借至 GPU 完成；描述符只借至 evaluate 返回。
        // output 为输出尺寸，其余为 SDK 查询的输入尺寸。
        // RGBA16F noisy/output/albedo/normal+roughness，R32F 正 view depth，RG16F motion。
        // 显式 specular motion 必需，distance 可全零且永不同时 tag 给 SDK。
        SeSlImage images[SE_SL_IMAGE_COUNT];
    } SeSlFrame;

    uint32_t se_sl_abi_version(void);
    // 未 bootstrap 时为空；成功后的 UTF-16 路径存储保留到进程退出。
    const wchar_t *se_sl_interposer_path(void);
    int32_t se_sl_bootstrap(const SeSlApplication *application);
    int32_t se_sl_shutdown(void);
    int32_t se_sl_create(const SeSlInit *init, const SeSlApplication *application, void **output);
    // width/height 为最终输出尺寸，size 返回档位对应的 SDK optimal 输入尺寸。
    int32_t se_sl_configure(void *context, uint32_t width, uint32_t height, uint32_t quality,
                            SeSlSize *size);
    // -100：SDK 异常，命令缓冲不可提交；其它失败保留 GENERAL，调用者可回退 noisy。
    int32_t se_sl_evaluate(void *context, const SeSlFrame *frame);
    int32_t se_sl_destroy(void *context);
    // 线程局部诊断，必须在下一次 bridge 调用前复制。
    const char *se_sl_last_error(void);

#ifdef __cplusplus
}
#endif
