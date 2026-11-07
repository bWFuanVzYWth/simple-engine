// 编译生产桥接，使用可失败的 SDK 函数表验证 FFI 退出边界；不加载驱动或 SDK DLL。
#include "bridge.cpp"
#include <cassert>

#ifdef NDEBUG
#error Bridge tests require assertions
#endif

namespace
{
sl::Result shutdown_result{sl::Result::eOk}, release_result{sl::Result::eOk};
bool throw_shutdown{};
unsigned shutdown_calls{};

sl::Result shutdown_stub()
{
    ++shutdown_calls;
    if (throw_shutdown)
        throw 1;
    return shutdown_result;
}

sl::Result release_stub(sl::Feature, const sl::ViewportHandle &)
{
    return release_result;
}

Context *context()
{
    auto *ctx = new Context();
    ctx->initialized = true;
    ctx->shutdown = shutdown_stub;
    ctx->free_resources = release_stub;
    return ctx;
}

void shutdown_failure_preserves_owner()
{
    auto *ctx = active = context();
    shutdown_result = static_cast<sl::Result>(1);
    assert(se_sl_destroy(ctx) != 0);
    assert(active == ctx && ctx->initialized);
    shutdown_result = sl::Result::eOk;
    assert(se_sl_destroy(ctx) == 0);
    assert(active == nullptr);
}

void resource_failure_preserves_owner()
{
    auto *ctx = active = context();
    ctx->evaluated = ctx->configured = true;
    release_result = static_cast<sl::Result>(1);
    assert(se_sl_destroy(ctx) != 0);
    assert(active == ctx && ctx->evaluated && ctx->configured);
    release_result = sl::Result::eOk;
    assert(se_sl_destroy(ctx) == 0);
    assert(active == nullptr);
}

void sdk_exception_cannot_cross_ffi()
{
    auto *ctx = active = context();
    throw_shutdown = true;
    assert(se_sl_destroy(ctx) == -4);
    assert(active == ctx && ctx->initialized);
    throw_shutdown = false;
    assert(se_sl_destroy(ctx) == 0);
}

void failed_initialization_cleanup_preserves_sdk_borrows()
{
    auto *ctx = context();
    const auto before = shutdown_calls;
    throw_shutdown = true;
    PendingContextDeleter{}(ctx);
    assert(failed_cleanup == ctx && ctx->initialized && shutdown_calls == before + 1);
    const SeSlApplication application{"test", "test"};
    assert(se_sl_bootstrap(&application) == -2);
    throw_shutdown = false;
    failed_cleanup = nullptr;
    PendingContextDeleter{}(ctx);
    assert(shutdown_calls == before + 2);
}
} // namespace

int main()
{
    shutdown_failure_preserves_owner();
    resource_failure_preserves_owner();
    sdk_exception_cannot_cross_ffi();
    failed_initialization_cleanup_preserves_sdk_borrows();
    std::puts("Streamline bridge: 4 failure-boundary tests passed");
}
