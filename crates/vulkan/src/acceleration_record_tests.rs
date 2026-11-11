use crate::{Error, TriangleBlasInput, TriangleInstance, interop::WgpuContext};

fn triangle() -> [[[f32; 3]; 3]; 1] {
    [[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]]
}

fn instance(geometry: crate::TriangleBlas) -> TriangleInstance {
    TriangleInstance {
        geometry,
        transform: [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]],
        custom_index: 0,
    }
}

#[test]
#[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
fn cancelled_blas_cannot_create_a_new_tlas() -> Result<(), Error> {
    crate::acceleration::validation::start();
    let mut context = pollster::block_on(WgpuContext::new())?;
    let triangles = triangle();
    let built = context.record_triangle_blas_batch(&[TriangleBlasInput {
        triangles: &triangles,
        opaque: true,
        opacity: None,
    }])?;
    let geometry = built.blases[0];
    context.cancel(built.commands)?;
    assert!(
        context
            .record_acceleration_instances(&[instance(geometry)])
            .is_err()
    );
    assert_eq!(
        context.native_ref().as_build_memory(),
        crate::AsBuildMemory::default()
    );
    drop(context);
    crate::acceleration::validation::finish()?;
    Ok(())
}

#[test]
#[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
fn consumer_before_producer_is_rejected_without_submission() -> Result<(), Error> {
    crate::acceleration::validation::start();
    let mut context = pollster::block_on(WgpuContext::new())?;
    let triangles = triangle();
    let blas = context.record_triangle_blas_batch(&[TriangleBlasInput {
        triangles: &triangles,
        opaque: true,
        opacity: None,
    }])?;
    let tlas = context.record_acceleration_instances(&[instance(blas.blases[0])])?;
    assert!(context.submit([tlas.commands.then(blas.commands)]).is_err());
    assert_eq!(context.last_submission_serial(), 0);
    assert_eq!(
        context.native_ref().as_build_memory(),
        crate::AsBuildMemory::default()
    );
    drop(context);
    crate::acceleration::validation::finish()?;
    Ok(())
}

#[test]
#[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
fn consumer_of_cancelled_producer_cannot_submit() -> Result<(), Error> {
    crate::acceleration::validation::start();
    let mut context = pollster::block_on(WgpuContext::new())?;
    let triangles = triangle();
    let blas = context.record_triangle_blas_batch(&[TriangleBlasInput {
        triangles: &triangles,
        opaque: true,
        opacity: None,
    }])?;
    let tlas = context.record_acceleration_instances(&[instance(blas.blases[0])])?;
    context.cancel(blas.commands)?;
    assert!(context.submit([tlas.commands]).is_err());
    assert_eq!(context.last_submission_serial(), 0);
    drop(context);
    crate::acceleration::validation::finish()?;
    Ok(())
}

#[test]
#[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
fn pending_build_resources_survive_collect_until_cancelled() -> Result<(), Error> {
    crate::acceleration::validation::start();
    let mut context = pollster::block_on(WgpuContext::new())?;
    let triangles = triangle();
    let blas = context.record_triangle_blas_batch(&[TriangleBlasInput {
        triangles: &triangles,
        opaque: true,
        opacity: None,
    }])?;
    let tlas = context.record_acceleration_instances(&[instance(blas.blases[0])])?;
    context.native().collect(&[])?;
    // 待执行记录仍保留新 AS；重新发布根能解析完整 TLAS→BLAS 依赖。
    context.native().collect(&[tlas.acceleration.into()])?;
    assert_eq!(context.native_ref().as_build_memory().pending_buffers, 4);
    context.cancel(blas.commands.then(tlas.commands))?;
    context.native().collect(&[])?;
    assert_eq!(
        context.native_ref().as_build_memory(),
        crate::AsBuildMemory::default()
    );
    assert!(context.native_ref().build_pool_idle_bytes() > 0);
    drop(context);
    crate::acceleration::validation::finish()?;
    Ok(())
}

#[test]
#[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
fn queued_blas_can_feed_a_later_tlas_submission() -> Result<(), Error> {
    crate::acceleration::validation::start();
    let mut context = pollster::block_on(WgpuContext::new())?;
    let triangles = triangle();
    let blas = context.record_triangle_blas_batch(&[TriangleBlasInput {
        triangles: &triangles,
        opaque: true,
        opacity: None,
    }])?;
    let geometry = blas.blases[0];
    let _blas_submission = context.submit([blas.commands])?;
    let tlas = context.record_acceleration_instances(&[instance(geometry)])?;
    let acceleration = tlas.acceleration;
    let submitted = context.submit([tlas.commands])?;
    assert_eq!(context.last_submission_serial(), 2);
    // 构建链未执行 host wait；发布根保留在途 TLAS 及其 BLAS 依赖。
    context.native().collect(&[acceleration.into()])?;
    crate::interop::wait(&context.native.device, Some(submitted))?;
    context.native().collect(&[acceleration.into()])?;
    assert_eq!(
        context.native_ref().as_build_memory(),
        crate::AsBuildMemory::default()
    );
    assert!(context.native_ref().build_pool_idle_bytes() > 0);
    drop(context);
    crate::acceleration::validation::finish()?;
    Ok(())
}

#[test]
#[ignore = "需要真实 KHR RT Vulkan；统一串行运行"]
fn build_temporaries_return_only_after_reported_completion() -> Result<(), Error> {
    crate::acceleration::validation::start();
    let mut context = pollster::block_on(WgpuContext::new())?;
    let triangles = triangle();
    let blas = context.record_triangle_blas_batch(&[TriangleBlasInput {
        triangles: &triangles,
        opaque: true,
        opacity: None,
    }])?;
    assert_eq!(blas.timing.submit_wait_ns, 0);
    assert_eq!(blas.timing.gpu_build_ns, None);
    let tlas = context.record_acceleration_instances(&[instance(blas.blases[0])])?;
    let before = context.native_ref().as_build_memory();
    assert!(before.pending_bytes > 0);
    assert_eq!(before.pending_buffers, 4);
    assert_eq!(context.native_ref().build_pool_idle_bytes(), 0);
    let acceleration = tlas.acceleration;
    let submitted = context.submit([blas.commands.then(tlas.commands)])?;
    // 只读 accessor 不收取 callback，因此此处验证的是已登记的真实提交有效期。
    let inflight = context.native_ref().as_build_memory();
    assert_eq!(inflight.pending_bytes, 0);
    assert_eq!(inflight.inflight_bytes, before.pending_bytes);
    assert_eq!(inflight.inflight_buffers, 4);
    crate::interop::wait(&context.native.device, Some(submitted))?;
    context.native().collect(&[acceleration.into()])?;
    assert_eq!(
        context.native_ref().as_build_memory(),
        crate::AsBuildMemory::default()
    );
    assert!(context.native_ref().build_pool_idle_bytes() > 0);
    drop(context);
    crate::acceleration::validation::finish()?;
    Ok(())
}

#[test]
#[ignore = "真实 Vulkan；在途复制保活大列，空根收齐驱动释放回执"]
fn retired_columns_survive_copy_and_return_their_allocation() -> Result<(), Error> {
    use crate::{interop, native::Buffer};
    crate::acceleration::validation::start();
    let mut ctx = pollster::block_on(interop::WgpuContext::new())?;
    ctx.native().prepare_quad_expansion()?;
    let initial = ctx.allocated_native_bytes();
    let src = Buffer::mapped_device_local(ctx.native(), 32 << 20)?;
    let dest = Buffer::mapped_device_local(ctx.native(), 32 << 20)?;
    src.write(ctx.native(), 0, &[7, 13, 29, 61])?;
    ctx.copy_buffer_prefixes(&[(src, dest, 4)])?;
    ctx.native().collect(&[dest.into()])?;
    interop::wait(&ctx.native_ref().device, None)?;
    let mut read = [0; 4];
    dest.read(ctx.native(), 0, &mut read)?;
    assert_eq!(read, [7, 13, 29, 61]);
    ctx.native().collect(&[])?;
    assert_eq!(ctx.allocated_native_bytes(), initial);
    drop(ctx);
    crate::acceleration::validation::finish()?;
    Ok(())
}
