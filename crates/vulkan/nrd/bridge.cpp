// 官方 NRD 调度和 SPIR-V 的 Vulkan 适配，纹理和宿主 command 始终由引擎拥有。
#include <algorithm>
#include <cmath>
#include <cstring>
#include <cstdio>
#include <memory>
#include "NRD.h"
#include "NRI.h"
#include "Extensions/NRIHelper.h"
#include "Extensions/NRIWrapperVK.h"
struct SdkFailure
{
};
#define NRD_INTEGRATION_ASSERT(expr, msg)                                                                              \
    do                                                                                                                 \
    {                                                                                                                  \
        if (!(expr))                                                                                                   \
        {                                                                                                              \
            std::fprintf(stderr, "NRD: %s\n", msg);                                                                    \
            throw SdkFailure{};                                                                                        \
        }                                                                                                              \
    } while (false)
#include "NRDIntegration.hpp"

namespace
{
using R = nrd::ResourceType;
struct Slot
{
    R resource;
    bool reflection;
};
// 同一实例中的两次 Denoise 共享临时池，各自按快照绑定运动、法线和深度。
constexpr Slot slots[] = {{R::IN_MV, false},
                          {R::IN_NORMAL_ROUGHNESS, false},
                          {R::IN_VIEWZ, false},
                          {R::IN_DIFF_SH0, false},
                          {R::IN_DIFF_SH1, false},
                          {R::IN_SPEC_SH0, true},
                          {R::IN_SPEC_SH1, true},
                          {R::IN_PENUMBRA, false},
                          {R::IN_DISOCCLUSION_THRESHOLD_MIX, false},
                          {R::OUT_DIFF_SH0, false},
                          {R::OUT_DIFF_SH1, false},
                          {R::OUT_SPEC_SH0, true},
                          {R::OUT_SPEC_SH1, true},
                          {R::OUT_SHADOW_TRANSLUCENCY, false},
                          {R::IN_MV, true},
                          {R::IN_NORMAL_ROUGHNESS, true},
                          {R::IN_VIEWZ, true},
                          {R::IN_DISOCCLUSION_THRESHOLD_MIX, true}};
constexpr nrd::Identifier ids[] = {1, 2, 3};
struct Image
{
    uint64_t handle;
    uint32_t format, pad;
};
struct Frame
{
    float projection[16], previousProjection[16], view[16], previousView[16];
    float jitter[2], previousJitter[2], sun[3], milliseconds;
    uint32_t index, reset;
};
struct Session
{
    nri::Device *device{};
    nri::CoreInterface core{};
    nri::WrapperVKInterface vk{};
    std::array<nri::Texture *, std::size(slots)> images{};
    nrd::Integration integration{};
    uint16_t size[2]{};
    float smoothedMs = 16.67f;
    ~Session()
    {
        integration.Destroy();
        for (auto *image : images)
            if (image)
                core.DestroyTexture(image);
        if (device)
            nri::nriDestroyDevice(device);
    }
};
} // namespace

extern "C" int engine_nrd_create(const uint64_t *handles, uint32_t family, uint32_t queueIndex, uint16_t width,
                                 uint16_t height, const Image *images, Session **result) noexcept
{
    try
    {
        auto s = std::make_unique<Session>();
        s->size[0] = width;
        s->size[1] = height;
        nri::QueueFamilyVKDesc queues{queueIndex + 1, nri::QueueType::GRAPHICS, family};
        nri::DeviceCreationVKDesc device{};
        device.vkInstance = reinterpret_cast<void *>(handles[0]);
        device.vkPhysicalDevice = reinterpret_cast<void *>(handles[1]);
        device.vkDevice = reinterpret_cast<void *>(handles[2]);
        device.queueFamilies = &queues;
        device.queueFamilyNum = 1;
        const char *extensions[] = {"VK_KHR_push_descriptor"};
        device.vkExtensions.deviceExtensionNum = 1;
        device.vkExtensions.deviceExtensions = extensions;
        device.minorVersion = 3;
        if (nri::nriCreateDeviceFromVKDevice(device, s->device) != nri::Result::SUCCESS)
            return 1;
        if (nri::nriGetInterface(*s->device, NRI_INTERFACE(nri::CoreInterface), &s->core) != nri::Result::SUCCESS ||
            nri::nriGetInterface(*s->device, NRI_INTERFACE(nri::WrapperVKInterface), &s->vk) != nri::Result::SUCCESS)
            return 2;
        for (size_t i = 0; i < s->images.size(); ++i)
        {
            nri::TextureVKDesc image{};
            image.vkImage = images[i].handle;
            image.vkFormat = images[i].format;
            image.vkImageType = 1;
            image.vkImageUsageFlags = 0xcu;
            image.width = width;
            image.height = height;
            image.depth = image.mipNum = image.layerNum = image.sampleNum = 1;
            if (s->vk.CreateTextureVK(*s->device, image, s->images[i]) != nri::Result::SUCCESS)
                return 3;
        }
        nrd::DenoiserDesc modes[] = {{ids[0], nrd::Denoiser::REBLUR_DIFFUSE_SH},
                                     {ids[1], nrd::Denoiser::SIGMA_SHADOW},
                                     {ids[2], nrd::Denoiser::REBLUR_SPECULAR_SH}};
        nrd::InstanceCreationDesc instance{};
        instance.denoisers = modes;
        instance.denoisersNum = static_cast<uint32_t>(std::size(modes));
        nrd::IntegrationCreationDesc config{};
        config.resourceWidth = width;
        config.resourceHeight = height;
        config.queuedFrameNum = 1;
        config.autoWaitForIdle = false;
        config.enableWholeLifetimeDescriptorCaching = true;
        if (s->integration.Recreate(config, instance, s->device) != nrd::Result::SUCCESS)
            return 4;
        *result = s.release();
        return 0;
    }
    catch (...)
    {
        return 5;
    }
}
extern "C" int engine_nrd_record(Session *s, uint64_t command, const Frame *frame) noexcept
{
    nri::CommandBuffer *wrapper = nullptr;
    try
    {
        auto &it = s->integration;
        it.NewFrame();
        nrd::CommonSettings common{};
        std::copy_n(frame->projection, 16, common.viewToClipMatrix);
        std::copy_n(frame->previousProjection, 16, common.viewToClipMatrixPrev);
        std::copy_n(frame->view, 16, common.worldToViewMatrix);
        std::copy_n(frame->previousView, 16, common.worldToViewMatrixPrev);
        for (int i = 0; i < 2; ++i)
        {
            common.resourceSize[i] = common.resourceSizePrev[i] = common.rectSize[i] = common.rectSizePrev[i] =
                s->size[i];
            common.motionVectorScale[i] = 1.0f / s->size[i];
            common.cameraJitter[i] = frame->jitter[i];
            common.cameraJitterPrev[i] = frame->previousJitter[i];
        }
        common.frameIndex = frame->index;
        common.motionVectorScale[2] = 1.0f;
        common.accumulationMode =
            frame->reset ? nrd::AccumulationMode::CLEAR_AND_RESTART : nrd::AccumulationMode::CONTINUE;
        common.isDisocclusionThresholdMixAvailable = true;
        // 高光专用输入已提供等价表面的运动。标记该类像素，使 SDK 采用输入运动锚点。
        common.cameraAttachedReflectionMaterialID = 3.0f;
        common.timeDeltaBetweenFrames = frame->milliseconds;
        if (it.SetCommonSettings(common) != nrd::Result::SUCCESS)
            return 6;
        float dt = std::clamp(frame->milliseconds, 1.f, 250.f);
        s->smoothedMs += (dt - s->smoothedMs) * (1.f - std::exp(-dt / 200.f));
        float fps = 1000.f / s->smoothedMs;
        nrd::ReblurSettings sh{};
        sh.maxAccumulatedFrameNum =
            std::min(nrd::REBLUR_MAX_HISTORY_FRAME_NUM, nrd::GetMaxAccumulatedFrameNum(2.f, fps));
        sh.maxStabilizedFrameNum = sh.maxAccumulatedFrameNum;
        sh.maxFastAccumulatedFrameNum = std::min(sh.maxAccumulatedFrameNum, nrd::GetMaxAccumulatedFrameNum(.12f, fps));
        sh.hitDistanceReconstructionMode = nrd::HitDistanceReconstructionMode::AREA_3X3;
        sh.minMaterialForDiffuse = 0;
        sh.minMaterialForSpecular = 0;
        nrd::SigmaSettings sigma{};
        std::copy_n(frame->sun, 3, sigma.lightDirection);
        sigma.maxStabilizedFrameNum =
            std::min(nrd::SIGMA_MAX_HISTORY_FRAME_NUM, nrd::GetMaxAccumulatedFrameNum(.25f, fps));
        if (it.SetDenoiserSettings(ids[0], &sh) != nrd::Result::SUCCESS ||
            it.SetDenoiserSettings(ids[1], &sigma) != nrd::Result::SUCCESS ||
            it.SetDenoiserSettings(ids[2], &sh) != nrd::Result::SUCCESS)
            return 7;
        nrd::ResourceSnapshot interior, reflection;
        interior.restoreInitialState = reflection.restoreInitialState = true;
        for (size_t i = 0; i < s->images.size(); ++i)
        {
            nrd::Resource r{};
            r.nri.texture = s->images[i];
            r.state = {nri::AccessBits::SHADER_RESOURCE_STORAGE, nri::Layout::GENERAL, nri::StageBits::ALL};
            auto &snapshot = slots[i].reflection ? reflection : interior;
            snapshot.SetResource(slots[i].resource, r);
        }
        nri::CommandBufferVKDesc desc{reinterpret_cast<void *>(command), nri::QueueType::GRAPHICS};
        if (s->vk.CreateCommandBufferVK(*s->device, desc, wrapper) != nri::Result::SUCCESS)
            return 8;
        it.Denoise(ids, 2, *wrapper, interior);
        it.Denoise(ids + 2, 1, *wrapper, reflection);
        s->core.DestroyCommandBuffer(wrapper);
        return 0;
    }
    catch (...)
    {
        if (wrapper)
            s->core.DestroyCommandBuffer(wrapper);
        return 9;
    }
}
extern "C" void engine_nrd_destroy(Session *session) noexcept
{
    delete session;
}
