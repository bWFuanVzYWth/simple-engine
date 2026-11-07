#include "bridge.h"
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <vulkan/vulkan.h>
#include <array>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include "sl.h"
#include "sl_dlss_d.h"
#include "sl_helpers_vk.h"

namespace
{
static_assert(sizeof(SeSlInit) == 32);
static_assert(sizeof(SeSlImage) == 48);
static_assert(sizeof(SeSlFrame) == 904);
static_assert(offsetof(SeSlFrame, images) == 472);
thread_local char error_text[1024]{};
thread_local bool evaluating{}, logged_error{};
std::mutex api_mutex;

int32_t fail(const char *message, int32_t code = -1)
{
    std::snprintf(error_text, sizeof(error_text), "%s (%d)", message, code);
    return code;
}

int32_t check(sl::Result result, const char *operation)
{
    return result == sl::Result::eOk ? 0 : fail(operation, static_cast<int32_t>(result));
}

void log_message(sl::LogType type, const char *message)
{
    if (evaluating && type == sl::LogType::eError)
    {
        logged_error = true;
        std::snprintf(error_text, sizeof(error_text), "Streamline: %s",
                      message ? message : "error");
    }
}

template <typename T> bool symbol(HMODULE module, const char *name, T *&function)
{
    function = reinterpret_cast<T *>(GetProcAddress(module, name));
    return function != nullptr;
}

struct Context
{
    HMODULE module{};
    bool initialized{}, interposed{}, evaluated{}, configured{};
    std::wstring directory, interposer_path;
    std::string engine_version, project_id;
    const wchar_t *plugin_path{};
    sl::ViewportHandle viewport{0};
    sl::DLSSDOptions options;
    SeSlSize input{}, output{};
    PFun_slInit *init{};
    PFun_slShutdown *shutdown{};
    PFun_slSetVulkanInfo *set_vulkan{};
    PFun_slIsFeatureSupported *supported{};
    PFun_slGetFeatureFunction *get_function{};
    PFun_slGetNewFrameToken *get_token{};
    PFun_slSetConstants *set_constants{};
    PFun_slEvaluateFeature *evaluate{};
    PFun_slFreeResources *free_resources{};
    PFun_slDLSSDSetOptions *set_options{};
    PFun_slDLSSDGetOptimalSettings *optimal{};
    ~Context()
    {
        if (module)
            FreeLibrary(module);
    }
};

Context *active{}, *bootstrapped{}, *failed_cleanup{};

// 初始化失败的本地 owner 也必须先成功关闭 SDK；异常时保留 SDK 引用的路径、身份与模块。
struct PendingContextDeleter
{
    void operator()(Context *ctx) const noexcept
    {
        if (ctx->initialized)
        {
            try
            {
                if (ctx->shutdown() != sl::Result::eOk)
                {
                    failed_cleanup = ctx;
                    std::fputs("Streamline initialization cleanup failed; retaining owner\n",
                               stderr);
                    return;
                }
            }
            catch (...)
            {
                failed_cleanup = ctx;
                std::fputs("Streamline initialization cleanup threw; retaining owner\n", stderr);
                return;
            }
            ctx->initialized = false;
        }
        delete ctx;
    }
};
using PendingContext = std::unique_ptr<Context, PendingContextDeleter>;

bool module_directory(std::wstring &result)
{
    HMODULE module{};
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                                GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                            reinterpret_cast<LPCWSTR>(&se_sl_create), &module))
        return false;
    wchar_t path[32768];
    DWORD count = GetModuleFileNameW(module, path, static_cast<DWORD>(std::size(path)));
    if (!count || count >= std::size(path))
        return false;
    result.assign(path, count);
    auto separator = result.find_last_of(L"\\/");
    if (separator == std::wstring::npos)
        return false;
    result.resize(separator);
    return true;
}

int32_t initialize(Context &ctx, bool interposed, const SeSlApplication &application)
{
    if (!module_directory(ctx.directory))
        return fail("Cannot find Streamline bridge module directory");
    ctx.interposer_path = ctx.directory + L"\\sl.interposer.dll";
    ctx.module = LoadLibraryExW(ctx.interposer_path.c_str(), nullptr,
                                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!ctx.module)
        return fail("Cannot load bundled sl.interposer.dll", -3);
#define LOAD(member, name)                                                                         \
    if (!symbol(ctx.module, #name, ctx.member))                                                    \
    return fail("Missing Streamline export " #name)
    LOAD(init, slInit);
    LOAD(shutdown, slShutdown);
    LOAD(set_vulkan, slSetVulkanInfo);
    LOAD(supported, slIsFeatureSupported);
    LOAD(get_function, slGetFeatureFunction);
    LOAD(get_token, slGetNewFrameToken);
    LOAD(set_constants, slSetConstants);
    LOAD(evaluate, slEvaluateFeature);
    LOAD(free_resources, slFreeResources);
#undef LOAD
    sl::Preferences preferences;
    ctx.plugin_path = ctx.directory.c_str();
    preferences.pathsToPlugins = &ctx.plugin_path;
    preferences.numPathsToPlugins = 1;
    const sl::Feature feature = sl::kFeatureDLSS_RR;
    preferences.featuresToLoad = &feature;
    preferences.numFeaturesToLoad = 1;
    preferences.flags = sl::PreferenceFlags::eDisableCLStateTracking |
                        sl::PreferenceFlags::eDisableDebugText |
                        sl::PreferenceFlags::eUseFrameBasedResourceTagging;
    if (!interposed)
        preferences.flags |= sl::PreferenceFlags::eUseManualHooking;
    preferences.engine = sl::EngineType::eCustom;
    ctx.engine_version = application.engine_version;
    ctx.project_id = application.project_id;
    preferences.engineVersion = ctx.engine_version.c_str();
    preferences.projectId = ctx.project_id.c_str();
    preferences.renderAPI = sl::RenderAPI::eVulkan;
    preferences.logMessageCallback = log_message;
    int32_t result = check(ctx.init(preferences, sl::kSDKVersion), "slInit");
    if (!result)
    {
        ctx.initialized = true;
        ctx.interposed = interposed;
    }
    return result;
}

template <typename T> int32_t feature_function(Context &ctx, const char *name, T *&function)
{
    void *address{};
    const auto result = ctx.get_function(sl::kFeatureDLSS_RR, name, address);
    function = reinterpret_cast<T *>(address);
    if (const auto error = check(result, name))
        return error;
    return function ? 0 : fail(name);
}

void copy_matrix(sl::float4x4 &destination, const float *source)
{
    std::memcpy(&destination, source, sizeof(destination));
}

void identity(sl::float4x4 &destination)
{
    for (uint32_t row = 0; row < 4; ++row)
        for (uint32_t col = 0; col < 4; ++col)
            (&destination[row].x)[col] = row == col ? 1.0f : 0.0f;
}

sl::DLSSDOptions make_options(uint32_t width, uint32_t height, sl::DLSSMode mode)
{
    sl::DLSSDOptions options;
    options.mode = mode;
    options.outputWidth = width;
    options.outputHeight = height;
    options.normalRoughnessMode = sl::DLSSDNormalRoughnessMode::ePacked;
    options.colorBuffersHDR = sl::Boolean::eTrue;
    options.alphaUpscalingEnabled = sl::Boolean::eFalse;
    options.dlaaPreset = options.qualityPreset = options.balancedPreset =
        options.performancePreset = options.ultraPerformancePreset = options.ultraQualityPreset =
            sl::DLSSDPreset::ePresetF;
    identity(options.worldToCameraView);
    identity(options.cameraViewToWorld);
    return options;
}

bool absent(const SeSlImage &image)
{
    const SeSlImage empty{};
    return std::memcmp(&image, &empty, sizeof(image)) == 0;
}

bool quality_mode(uint32_t quality, sl::DLSSMode &mode)
{
    switch (quality)
    {
    case SE_SL_DLAA:
        mode = sl::DLSSMode::eDLAA;
        break;
    case SE_SL_QUALITY:
        mode = sl::DLSSMode::eMaxQuality;
        break;
    case SE_SL_BALANCED:
        mode = sl::DLSSMode::eBalanced;
        break;
    case SE_SL_PERFORMANCE:
        mode = sl::DLSSMode::eMaxPerformance;
        break;
    case SE_SL_ULTRA_PERFORMANCE:
        mode = sl::DLSSMode::eUltraPerformance;
        break;
    default:
        return false;
    }
    return true;
}

bool valid_frame(const Context &ctx, const SeSlFrame &frame)
{
    if (!frame.command || frame.reset > 1)
        return false;
    constexpr size_t count = (offsetof(SeSlFrame, images) - offsetof(SeSlFrame, jitter)) / 4;
    const auto *values = reinterpret_cast<const char *>(&frame) + offsetof(SeSlFrame, jitter);
    for (size_t index = 0; index < count; ++index)
    {
        float value;
        std::memcpy(&value, values + index * 4, 4);
        if (!std::isfinite(value))
            return false;
    }
    if (frame.near_plane <= 0 || frame.far_plane <= frame.near_plane || frame.fov <= 0 ||
        frame.fov >= 3.141593f || frame.aspect <= 0)
        return false;
    constexpr uint32_t formats[] = {
        VK_FORMAT_R16G16B16A16_SFLOAT, VK_FORMAT_R32_SFLOAT,          VK_FORMAT_R16G16_SFLOAT,
        VK_FORMAT_R16G16B16A16_SFLOAT, VK_FORMAT_R16G16B16A16_SFLOAT, VK_FORMAT_R16G16B16A16_SFLOAT,
        VK_FORMAT_R16G16B16A16_SFLOAT, VK_FORMAT_R32_SFLOAT,          VK_FORMAT_R16G16_SFLOAT};
    for (uint32_t index = 0; index < SE_SL_IMAGE_COUNT; ++index)
    {
        if (index == SE_SL_SPECULAR_HIT_DISTANCE && absent(frame.images[index]))
            continue;
        const auto &image = frame.images[index];
        const auto size = index == SE_SL_OUTPUT ? ctx.output : ctx.input;
        if (!image.image || !image.view || image.width != size.width ||
            image.height != size.height || image.format != formats[index] ||
            image.layout != VK_IMAGE_LAYOUT_GENERAL || image.reserved ||
            !(image.usage & VK_IMAGE_USAGE_SAMPLED_BIT) ||
            !(image.usage & VK_IMAGE_USAGE_STORAGE_BIT))
            return false;
        if (index != SE_SL_OUTPUT && image.image == frame.images[SE_SL_OUTPUT].image)
            return false;
    }
    return true;
}

sl::Constants make_constants(const SeSlFrame &frame, SeSlSize size)
{
    sl::Constants constants;
    copy_matrix(constants.cameraViewToClip, frame.view_to_clip);
    copy_matrix(constants.clipToCameraView, frame.clip_to_view);
    copy_matrix(constants.clipToPrevClip, frame.clip_to_previous);
    copy_matrix(constants.prevClipToClip, frame.previous_to_clip);
    identity(constants.clipToLensClip);
    constants.jitterOffset = {frame.jitter[0], frame.jitter[1]};
    constants.mvecScale = {1.0f / size.width, 1.0f / size.height};
    constants.cameraPinholeOffset = {0, 0};
    constants.cameraPos = {frame.position[0], frame.position[1], frame.position[2]};
    constants.cameraUp = {frame.up[0], frame.up[1], frame.up[2]};
    constants.cameraRight = {frame.right[0], frame.right[1], frame.right[2]};
    constants.cameraFwd = {frame.forward[0], frame.forward[1], frame.forward[2]};
    constants.cameraNear = frame.near_plane;
    constants.cameraFar = frame.far_plane;
    constants.cameraFOV = frame.fov;
    constants.cameraAspectRatio = frame.aspect;
    constants.depthInverted = sl::Boolean::eFalse;
    constants.cameraMotionIncluded = sl::Boolean::eTrue;
    constants.motionVectors3D = sl::Boolean::eFalse;
    constants.motionVectorsJittered = sl::Boolean::eFalse;
    constants.motionVectorsDilated = sl::Boolean::eFalse;
    constants.orthographicProjection = sl::Boolean::eFalse;
    constants.reset = frame.reset ? sl::Boolean::eTrue : sl::Boolean::eFalse;
    return constants;
}

sl::Resource make_resource(const SeSlImage &image)
{
    sl::Resource resource;
    resource.type = sl::ResourceType::eTex2d;
    resource.native = reinterpret_cast<void *>(image.image);
    resource.view = reinterpret_cast<void *>(image.view);
    resource.memory = reinterpret_cast<void *>(image.memory);
    resource.state = image.layout;
    resource.width = image.width;
    resource.height = image.height;
    resource.nativeFormat = image.format;
    resource.mipLevels = resource.arrayLayers = 1;
    resource.flags = 0;
    resource.usage = image.usage;
    return resource;
}
} // namespace

extern "C" uint32_t se_sl_abi_version()
{
    return 3;
}
extern "C" const char *se_sl_last_error()
{
    return error_text;
}

// bootstrap context 保留到进程退出；路径初始化后不再更改。
extern "C" const wchar_t *se_sl_interposer_path()
try
{
    std::lock_guard lock(api_mutex);
    return bootstrapped && bootstrapped->initialized ? bootstrapped->interposer_path.c_str()
                                                     : nullptr;
}
catch (...)
{
    fail("Exception while accessing Streamline interposer path", -4);
    return nullptr;
}

extern "C" int32_t se_sl_bootstrap(const SeSlApplication *application)
try
{
    std::lock_guard lock(api_mutex);
    if (failed_cleanup)
        return fail("Previous Streamline cleanup failed; restart required", -2);
    if (!application || !application->engine_version || !*application->engine_version ||
        !application->project_id || !*application->project_id)
        return fail("Invalid Streamline application identity");
    if (bootstrapped)
    {
        if (bootstrapped->engine_version != application->engine_version ||
            bootstrapped->project_id != application->project_id)
            return fail("Streamline application identity mismatch");
        return bootstrapped->initialized ? 0 : fail("Streamline already shut down");
    }
    if (active)
        return fail("Streamline bootstrap must precede host Vulkan creation", -2);
    // early interposer 不接受 layer-only instance extension；同步验证由同一层环境配置启用。
    SetLastError(ERROR_SUCCESS);
    if (!GetEnvironmentVariableW(L"VK_LAYER_VALIDATE_SYNC", nullptr, 0) &&
        GetLastError() == ERROR_ENVVAR_NOT_FOUND &&
        !SetEnvironmentVariableW(L"VK_LAYER_VALIDATE_SYNC", L"1"))
        return fail("Cannot enable Khronos synchronization validation", -3);
    PendingContext ctx(new Context());
    const auto result = initialize(*ctx, true, *application);
    if (!result)
        bootstrapped = ctx.release();
    return result;
}
catch (...)
{
    return fail("Exception during Streamline bootstrap", -4);
}

extern "C" int32_t se_sl_shutdown()
try
{
    std::lock_guard lock(api_mutex);
    if (active)
        return fail("RR owner must retire before host device shutdown", -2);
    if (!bootstrapped || !bootstrapped->initialized)
        return 0;
    const auto result = check(bootstrapped->shutdown(), "slShutdown");
    if (!result)
        bootstrapped->initialized = false;
    // 保留 proxy 模块至进程退出；宿主尚需用它执行 vkDestroyDevice/Instance。
    return result;
}
catch (...)
{
    return fail("Exception during Streamline shutdown", -4);
}

extern "C" int32_t se_sl_create(const SeSlInit *init, const SeSlApplication *application,
                                void **output)
try
{
    std::lock_guard lock(api_mutex);
    if (output)
        *output = nullptr;
    if (failed_cleanup)
        return fail("Previous Streamline cleanup failed; restart required", -2);
    if (!init || !output || !init->device || !init->instance || !init->physical_device ||
        !application || !application->engine_version || !*application->engine_version ||
        !application->project_id || !*application->project_id)
        return fail("Invalid Streamline initialization");
    if (active)
        return fail("Only one RR owner can be active", -2);
    {
        PendingContext owned;
        Context *ctx = bootstrapped;
        int32_t result;
        if (!ctx)
        {
            owned.reset(new Context());
            ctx = owned.get();
            if ((result = initialize(*ctx, false, *application)))
                return result;
        }
        if (!ctx->initialized)
            return fail("Streamline already shut down");
        if (ctx->engine_version != application->engine_version ||
            ctx->project_id != application->project_id)
            return fail("Streamline application identity mismatch");
        sl::AdapterInfo adapter;
        adapter.vkPhysicalDevice = reinterpret_cast<void *>(init->physical_device);
        if ((result = check(ctx->supported(sl::kFeatureDLSS_RR, adapter), "DLSS RR unsupported")))
            return result;
        sl::VulkanInfo info;
        info.instance = reinterpret_cast<VkInstance>(init->instance);
        info.physicalDevice = reinterpret_cast<VkPhysicalDevice>(init->physical_device);
        info.device = reinterpret_cast<VkDevice>(init->device);
        info.graphicsQueueFamily = info.computeQueueFamily = init->queue_family;
        info.graphicsQueueIndex = info.computeQueueIndex = init->queue_index;
        // SDK 此接口只供 manual hooking；proxy 的 vkCreateDevice 已初始化 plugin。
        if (!ctx->interposed && (result = check(ctx->set_vulkan(info), "slSetVulkanInfo")))
            return result;
        if ((result = feature_function(*ctx, "slDLSSDSetOptions", ctx->set_options)) ||
            (result = feature_function(*ctx, "slDLSSDGetOptimalSettings", ctx->optimal)))
            return result;
        active = ctx;
        if (owned)
            owned.release();
        *output = active;
        return 0;
    }
}
catch (...)
{
    return fail("Exception while attaching Streamline", -4);
}

extern "C" int32_t se_sl_configure(void *context, uint32_t width, uint32_t height, uint32_t quality,
                                   SeSlSize *size)
try
{
    std::lock_guard lock(api_mutex);
    if (!context || context != active || !width || !height || !size)
        return fail("Invalid RR configuration");
    sl::DLSSMode mode;
    if (!quality_mode(quality, mode))
        return fail("Invalid RR quality mode");
    auto &ctx = *active;
    auto options = make_options(width, height, mode);
    sl::DLSSDOptimalSettings optimal;
    int32_t result = check(ctx.optimal(options, optimal), "slDLSSDGetOptimalSettings");
    if (result)
        return result;
    const SeSlSize input{optimal.optimalRenderWidth, optimal.optimalRenderHeight};
    if (!input.width || !input.height || input.width > width || input.height > height ||
        (mode == sl::DLSSMode::eDLAA && (input.width != width || input.height != height)))
        return fail("Invalid DLSS RR optimal input extent");
    if (ctx.evaluated)
    {
        if ((result =
                 check(ctx.free_resources(sl::kFeatureDLSS_RR, ctx.viewport), "slFreeResources")))
            return result;
        ctx.evaluated = false;
    }
    if ((result = check(ctx.set_options(ctx.viewport, options), "slDLSSDSetOptions")))
        return result;
    ctx.options = options;
    ctx.input = input;
    ctx.output = {width, height};
    ctx.configured = true;
    *size = ctx.input;
    return 0;
}
catch (...)
{
    return fail("Exception during Streamline configuration", -4);
}

extern "C" int32_t se_sl_evaluate(void *context, const SeSlFrame *frame)
try
{
    std::lock_guard lock(api_mutex);
    if (!context || context != active || !frame || !active->configured ||
        !valid_frame(*active, *frame))
        return fail("Invalid RR frame resources or constants");
    auto &ctx = *active;
    copy_matrix(ctx.options.worldToCameraView, frame->world_to_view);
    copy_matrix(ctx.options.cameraViewToWorld, frame->view_to_world);
    int32_t result = check(ctx.set_options(ctx.viewport, ctx.options), "slDLSSDSetOptions");
    if (result)
        return result;
    sl::FrameToken *token{};
    if ((result = check(ctx.get_token(token, nullptr), "slGetNewFrameToken")))
        return result;
    if (!token)
        return fail("Streamline returned a null frame token");
    const auto constants = make_constants(*frame, ctx.input);
    if ((result = check(ctx.set_constants(constants, *token, ctx.viewport), "slSetConstants")))
        return result;
    constexpr sl::BufferType types[] = {sl::kBufferTypeScalingInputColor,
                                        sl::kBufferTypeLinearDepth,
                                        sl::kBufferTypeMotionVectors,
                                        sl::kBufferTypeNormalRoughness,
                                        sl::kBufferTypeAlbedo,
                                        sl::kBufferTypeSpecularAlbedo,
                                        sl::kBufferTypeScalingOutputColor,
                                        sl::kBufferTypeSpecularHitDistance,
                                        sl::kBufferTypeSpecularMotionVectors};
    std::array<sl::Resource, SE_SL_IMAGE_COUNT> resources;
    std::array<sl::ResourceTag, SE_SL_IMAGE_COUNT> tags;
    std::array<const sl::BaseStructure *, SE_SL_IMAGE_COUNT + 1> inputs{};
    inputs[0] = &ctx.viewport;
    uint32_t count = 1;
    for (uint32_t index = 0; index < SE_SL_IMAGE_COUNT; ++index)
    {
        if (index == SE_SL_SPECULAR_HIT_DISTANCE)
            continue;
        resources[index] = make_resource(frame->images[index]);
        const sl::Extent extent{0, 0, frame->images[index].width, frame->images[index].height};
        tags[index] = sl::ResourceTag(&resources[index], types[index],
                                      sl::ResourceLifecycle::eValidUntilEvaluate, &extent);
        inputs[count++] = &tags[index];
    }
    ctx.evaluated = true;
    evaluating = true;
    logged_error = false;
    sl::Result status;
    try
    {
        status = ctx.evaluate(sl::kFeatureDLSS_RR, *token, inputs.data(), count,
                              reinterpret_cast<sl::CommandBuffer *>(frame->command));
    }
    catch (...)
    {
        evaluating = false;
        return fail("SDK exception; command buffer must not be submitted", -100);
    }
    evaluating = false;
    if (status == sl::Result::eErrorExceptionHandler)
        return fail("SDK exception; command buffer must not be submitted", -100);
    if (status != sl::Result::eOk)
        return check(status, "slEvaluateFeature(DLSS RR)");
    // 2.14.1 plugin 不传递 NGX bool，必须检查同步错误回调。
    return logged_error ? -5 : 0;
}
catch (...)
{
    evaluating = false;
    return fail("SDK exception; command buffer must not be submitted", -100);
}

extern "C" int32_t se_sl_destroy(void *context)
try
{
    std::lock_guard lock(api_mutex);
    if (!context)
        return 0;
    if (context != active)
        return fail("Invalid RR owner");
    auto *ctx = active;
    int32_t result = 0;
    if (ctx->evaluated)
        result = check(ctx->free_resources(sl::kFeatureDLSS_RR, ctx->viewport), "slFreeResources");
    if (result)
        return result; // 失败时保留 owner，不能证明 SDK 私有资源已释放。
    ctx->evaluated = ctx->configured = false;
    if (ctx != bootstrapped)
    {
        result = check(ctx->shutdown(), "slShutdown");
        if (result)
            return result;
        ctx->initialized = false;
        active = nullptr;
        delete ctx;
    }
    else
        active = nullptr;
    return result;
}
catch (...)
{
    return fail("Exception during Streamline resource release", -4);
}
