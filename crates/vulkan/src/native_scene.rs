//! BLAS/TLAS场景准备与发布；输入仅借用，失败保留既有资源。
use super::*;

impl Context {
    /// 同步构建 BLAS；输入仅在构建期间借用。
    pub fn triangle_blas(
        &mut self,
        triangles: &[[[f32; 3]; 3]],
        opaque: bool,
    ) -> Result<crate::TriangleBlas, Error> {
        let data = BlasData::native(&self.device, triangles, opaque)?;
        Ok(crate::TriangleBlas {
            key: self.insert(Data::Blas(data))?,
        })
    }

    /// 同步构建一批独立 BLAS；共用一份输入与互不重叠的 scratch，只提交和等待一次。
    ///
    /// 支持借入的原生设备与 wgpu 适配设备，输入在返回后即可释放。每项显式指定
    /// opaque 覆盖和可选 OMM，输出次序与输入一致；空批不提交。新标识全部成功后才返回，
    /// 失败保留既有资源；临时分配与独立 backing 按 Context 的池上限复用。
    /// 原生宿主继续遵守 [`Self::from_raw`] 的队列宿主访问串行化与同队列提交契约。
    /// # Errors
    /// 非有限、空几何、数量超限、跨设备或无效 OMM、分配、录制及提交/完成失败保留来源。
    pub fn triangle_blas_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
    ) -> Result<crate::TriangleBlasBatch, Error> {
        self.triangle_blas_batch_timed(inputs, false)
    }

    pub(crate) fn triangle_blas_batch_timed(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
        timestamps: bool,
    ) -> Result<crate::TriangleBlasBatch, Error> {
        self.prepare_triangle_blas_batch(
            crate::acceleration::Sources::Triangles(inputs),
            timestamps,
            false,
        )
        .map(|(batch, _)| batch)
    }

    pub(crate) fn record_triangle_blas_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
    ) -> Result<crate::interop::PreparedBlasBatch, Error> {
        self.record_blas_inputs(crate::acceleration::Sources::Triangles(inputs))
    }

    pub(crate) fn record_blas_inputs(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
    ) -> Result<crate::interop::PreparedBlasBatch, Error> {
        let (batch, commands) = self.prepare_triangle_blas_batch(inputs, false, true)?;
        Ok(crate::interop::PreparedBlasBatch {
            blases: batch.blases,
            commands: commands.expect("录制型 BLAS 始终返回命令"),
            timing: batch.timing,
        })
    }

    pub(super) fn resolve_blas_inputs<'a>(
        device: &Device,
        life: &'a Life<Native, BuildBuffers>,
        inputs: crate::acceleration::Sources<'a>,
    ) -> Result<Vec<ResolvedBlas<'a>>, Error> {
        let mut resolved = Vec::new();
        resolved.try_reserve_exact(inputs.len())?;
        use crate::acceleration::{Positions, Sources};
        let mut writes = Vec::new();
        for i in 0..inputs.len() {
            let mut destinations = [0; 4];
            let mut region = [0; 3];
            let (positions, opaque, opacity) = match inputs {
                Sources::Triangles(v) => (
                    Positions::Triangles(v[i].triangles),
                    v[i].opaque,
                    v[i].opacity,
                ),
                Sources::Quads(v) | Sources::Uploaded(v, _) => {
                    let src = &v[i];
                    region = src.output.region;
                    let count = src.quads.positions().headers().len() as u64;
                    for (j, (target, stride, base)) in src.output.columns().into_iter().enumerate()
                    {
                        let Some(target) = target else { continue };
                        life.active(target.key)?;
                        let Data::Buffer(data) = life.resource(target.key)? else {
                            return Err(Error::InvalidArgument("quad 输出不是 buffer"));
                        };
                        let start = base + u64::from(src.output.first) * stride;
                        let end = start + count * stride;
                        if end > target.len() {
                            return Err(Error::InvalidArgument("quad 输出槽超过设备缓冲范围"));
                        }
                        writes.try_reserve(1)?;
                        writes.push((target.key, start, end));
                        destinations[j] = data.address(device) + start;
                    }
                    (Positions::Quads(src.quads), src.opaque, src.opacity)
                }
            };
            let opacity = if let Some((id, indices)) = opacity {
                life.active(id.key)?;
                let Data::Micromap(map) = life.resource(id.key)? else {
                    return Err(Error::InvalidArgument("OMM 标识类型无效"));
                };
                Some((map, id, indices))
            } else {
                None
            };
            resolved.push(ResolvedBlas {
                positions,
                uploaded: if let Sources::Uploaded(_, staged) = inputs {
                    let b = staged[i].buffer;
                    life.active(b.key)?;
                    let Data::Buffer(data) = life.resource(b.key)? else {
                        return Err(Error::InvalidArgument("地形源不是设备缓冲"));
                    };
                    Some(data.address(device))
                } else {
                    None
                },
                destinations,
                region,
                opaque,
                opacity,
            });
        }
        writes.sort_unstable();
        if writes
            .windows(2)
            .any(|w| w[0].0 == w[1].0 && w[0].2 > w[1].1)
        {
            return Err(Error::InvalidArgument("quad 输出切片重叠"));
        }
        Ok(resolved)
    }

    pub(super) fn prepare_triangle_blas_batch(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
        timestamps: bool,
        record_only: bool,
    ) -> Result<
        (
            crate::TriangleBlasBatch,
            Option<crate::interop::CommandBuffer>,
        ),
        Error,
    > {
        let started = std::time::Instant::now();
        let resolved = Self::resolve_blas_inputs(&self.device, &self.life, inputs)?;
        let mut access = Vec::new();
        let mut uses = Vec::new();
        if let crate::acceleration::Sources::Quads(v) | crate::acceleration::Sources::Uploaded(v, _) =
            inputs
            && record_only
        {
            let mut buffers = Vec::new();
            buffers.try_reserve_exact(
                v.len()
                    .checked_mul(5)
                    .ok_or(Error::InvalidArgument("输出列数量溢出"))?,
            )?;
            buffers.extend(
                v.iter()
                    .flat_map(|v| v.output.columns())
                    .filter_map(|(b, _, _)| b),
            );
            if let crate::acceleration::Sources::Uploaded(_, staged) = inputs {
                buffers.extend(staged.iter().map(|s| s.buffer));
            }
            buffers.sort_unstable_by_key(|b| b.key);
            buffers.dedup_by_key(|b| b.key);
            access.try_reserve_exact(buffers.len())?;
            uses.try_reserve_exact(buffers.len())?;
            for b in buffers {
                access.push(Op::Access(b.into()));
                uses.push(b.key);
            }
        }
        let mut blases = Vec::new();
        let mut roots = Vec::new();
        blases.try_reserve_exact(inputs.len())?;
        if record_only {
            roots.try_reserve_exact(inputs.len())?;
        }
        let validation_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        let output = BlasData::batch(
            &self.device,
            &resolved,
            timestamps,
            if record_only {
                BatchMode::Wgpu
            } else {
                BatchMode::Sync
            },
            &mut self.as_pool,
            self.quad_expansion.as_ref(),
        )?;
        drop(resolved);
        let mut timing = output.timing;
        let mut pending = output.recorded;
        let mut data = output.blases.into_iter();
        while let Some(blas) = data.next() {
            match self.insert(Data::Blas(blas)) {
                Ok(key) => {
                    blases.push(crate::TriangleBlas { key });
                    if record_only {
                        roots.push(key);
                    }
                }
                Err(error) => {
                    if let Some((_, buffers)) = pending.take() {
                        buffers.recycle(&self.device, &mut self.as_pool);
                    }
                    for remaining in data {
                        remaining.recycle(&self.device, &mut self.as_pool);
                    }
                    for blas in blases {
                        self.discard(blas.key);
                    }
                    return Err(error);
                }
            }
        }
        let commands = if let Some((raw, buffers)) = pending {
            match self.insert_build_record(Record::new(access, uses, roots, buffers)) {
                Ok(key) => Some(crate::interop::CommandBuffer::recorded(raw, key)),
                Err(error) => {
                    for blas in blases {
                        self.discard(blas.key);
                    }
                    return Err(error);
                }
            }
        } else {
            None
        };
        timing.validation_ns = timing.validation_ns.saturating_add(validation_ns);
        timing.pool_idle_bytes = self.as_pool.stats().idle_bytes;
        timing.total_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        Ok((crate::TriangleBlasBatch { blases, timing }, commands))
    }

    /// 以一次提交和一次完成等待构建新 BLAS 及其场景 TLAS。
    ///
    /// 实例可引用已有 BLAS 或同批输入索引；实例顺序与 BLAS 输入顺序分别保留。调用期间
    /// 保留全部输入和 scratch，实际完成后归池。宿主须按 [`Self::from_raw`] 独占队列。
    ///
    /// # Errors
    /// 设备不支持、来源无效或分配、录制、提交及等待失败时不返回任何新标识，已有资源有效。
    pub fn triangle_scene_batch(
        &mut self,
        inputs: &[crate::TriangleBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<crate::TriangleSceneBatch, Error> {
        self.scene_batch(
            crate::acceleration::Sources::Triangles(inputs),
            instances,
            false,
            None,
        )
    }

    /// 在同一命令中把紧凑 quad 展开到设备临时缓冲，构建 BLAS，再构建 TLAS。
    /// 同步宿主返回时均已完成，输入顺序和两个半面的编号保持不变。
    /// # Errors
    /// 展开管线未准备、输入无效、设备或分配失败时，保留已有场景。
    pub fn quad_scene_batch(
        &mut self,
        inputs: &[crate::QuadBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<crate::TriangleSceneBatch, Error> {
        self.scene_batch(
            crate::acceleration::Sources::Quads(inputs),
            instances,
            false,
            None,
        )
    }

    /// 向借入的原生宿主队列提交地形展开、BLAS 与 TLAS，后续追踪按同队列顺序观察新场景。
    /// GPU 完成 fence 之前由 Context 保活输入、scratch、输出及全部加速结构。
    /// # Errors
    /// 输入、设备或提交失败保留上个场景。只支持由调用方独占队列的原生宿主。
    pub fn enqueue_quad_scene(
        &mut self,
        inputs: &[crate::QuadBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<crate::TriangleSceneBatch, Error> {
        self.scene_batch(
            crate::acceleration::Sources::Quads(inputs),
            instances,
            true,
            None,
        )
    }
    /// 按本帧可用字节提交区域。超额包分帧上传；返回 None 时旧覆盖继续渲染且不能提交候选目录。
    /// # Errors
    /// 同 [`Self::enqueue_quad_scene`]；帧预算必须来自此渲染所有者，暂存 buffer 参与 collect。
    pub fn stream_quad_scene(
        &mut self,
        uploads: &mut crate::TerrainUploads,
        available: usize,
        inputs: &[crate::QuadBlasInput<'_>],
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
    ) -> Result<Option<crate::TriangleSceneBatch>, Error> {
        use crate::{acceleration::Sources, terrain_upload::Progress};
        let tail = instances
            .len()
            .checked_mul(64)
            .ok_or(Error::InvalidArgument("TLAS 实例容量溢出"))?;
        let staged = uploads.advance(self, inputs, available, tail)?;
        let scene = match staged {
            Progress::Direct => self.scene_batch(Sources::Quads(inputs), instances, true, None)?,
            Progress::Staged { start, buffers } => {
                if !buffers.is_empty() {
                    let window = &inputs[start..start + buffers.len()];
                    let (batch, _, _) = self.scene_parts(
                        Sources::Uploaded(window, &buffers),
                        None,
                        true,
                        None,
                        None,
                    )?;
                    uploads.remember(window, &batch)?;
                }
                let Some(built) = uploads.ready(inputs.len(), tail)? else {
                    return Ok(None);
                };
                let mut current = Vec::new();
                current.try_reserve_exact(instances.len())?;
                let mut existing = Vec::new();
                existing.try_reserve_exact(instances.len())?;
                for i in instances {
                    let id = match i.geometry {
                        crate::BatchBlas::Existing(id) => id,
                        crate::BatchBlas::New(k) => *built
                            .blases
                            .get(k)
                            .ok_or(Error::InvalidArgument("新BLAS下标越界"))?,
                    };
                    current.push(crate::TriangleInstance {
                        geometry: id,
                        transform: i.transform,
                        custom_index: i.custom_index,
                    });
                    existing.push(crate::TriangleInstance {
                        geometry: crate::BatchBlas::Existing(id),
                        transform: i.transform,
                        custom_index: i.custom_index,
                    });
                }
                let buffer = if tail > uploads.limit() {
                    let bytes = self.instance_payload(&current)?;
                    let Some(b) = uploads.instance_buffer(self, &bytes)? else {
                        return Ok(None);
                    };
                    Some(b)
                } else {
                    None
                };
                let mut scene =
                    self.scene_batch(Sources::Triangles(&[]), &existing, true, buffer)?;
                scene.blas = built;
                scene
            }
        };
        uploads.clear();
        Ok(Some(scene))
    }

    pub(super) fn scene_batch(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
        queued: bool,
        instance_buffer: Option<Buffer>,
    ) -> Result<crate::TriangleSceneBatch, Error> {
        let (blas, top, tlas_prepare_ns) =
            self.scene_parts(inputs, Some(instances), queued, instance_buffer, None)?;
        Ok(crate::TriangleSceneBatch {
            blas,
            top: top.expect("请求了 TLAS"),
            tlas_prepare_ns,
        })
    }

    /// 在原生队列上异步构建实例 TLAS。调用期间借用两个等长数组，每个实例的八位掩码
    /// 与追踪光线的掩码按位相交。全部 BLAS 须已发布，后续同队列命令可直接消费结果。
    /// # Errors
    /// 长度不一致、无效实例、设备或提交失败保留来源。
    pub fn enqueue_masked_instances(
        &mut self,
        instances: &[crate::TriangleInstance<crate::BatchBlas>],
        masks: &[u8],
    ) -> Result<crate::AccelerationStructure, Error> {
        if masks.len() != instances.len() {
            return Err(Error::InvalidArgument("实例与可见性掩码数量不同"));
        }
        let (_, top, _) = self.scene_parts(
            crate::acceleration::Sources::Triangles(&[]),
            Some(instances),
            true,
            None,
            Some(masks),
        )?;
        Ok(top.expect("本次请求了 TLAS"))
    }

    pub(super) fn scene_parts(
        &mut self,
        inputs: crate::acceleration::Sources<'_>,
        top_input: Option<&[crate::TriangleInstance<crate::BatchBlas>]>,
        queued: bool,
        instance_buffer: Option<Buffer>,
        masks: Option<&[u8]>,
    ) -> Result<
        (
            crate::TriangleBlasBatch,
            Option<crate::AccelerationStructure>,
            u64,
        ),
        Error,
    > {
        let instances = top_input.unwrap_or(&[]);
        self.device.ensure_active()?;
        if queued {
            self.poll_terrain()?;
        }
        for instance in instances {
            match instance.geometry {
                crate::BatchBlas::Existing(blas) => {
                    self.active(blas.key)?;
                    self.producer_submitted(blas.key)?;
                }
                crate::BatchBlas::New(index) if index >= inputs.len() => {
                    return Err(Error::InvalidArgument("实例的新 BLAS 索引超出批输入"));
                }
                crate::BatchBlas::New(_) => {}
            }
        }
        let mut blases = Vec::new();
        blases.try_reserve_exact(inputs.len())?;
        let mut top = None;
        let mut blas_buffers = BuildBuffers::default();
        let mut tlas_buffers = BuildBuffers::default();
        let mut timing = crate::BuildTiming::default();
        let mut tlas_prepare_ns = 0;
        let mut preparation_ns = 0;
        let started = std::time::Instant::now();
        let elapsed = |instant: std::time::Instant| {
            u64::try_from(instant.elapsed().as_nanos()).unwrap_or(u64::MAX)
        };
        let instance_address = instance_buffer
            .map(|b| -> Result<u64, Error> { Ok(b.data(self)?.address(&self.device)) })
            .transpose()?;
        let device = &self.device;
        let pool = &mut self.as_pool;
        let life = &mut self.life;
        let encode = |command| {
            let preparation = std::time::Instant::now();
            let resolved = Self::resolve_blas_inputs(device, life, inputs)?;
            let validation_ns = elapsed(preparation);
            let output = BlasData::batch(
                device,
                &resolved,
                false,
                BatchMode::Raw(command),
                pool,
                self.quad_expansion.as_ref(),
            )?;
            drop(resolved);
            timing = output.timing;
            timing.validation_ns = timing.validation_ns.saturating_add(validation_ns);
            blas_buffers = output.raw_buffers;
            let mut data = output.blases.into_iter();
            while let Some(blas) = data.next() {
                match life.insert_resource(Data::Blas(blas)) {
                    Ok(key) => blases.push(crate::TriangleBlas { key }),
                    Err((error, blas)) => {
                        blas.recycle(device, pool);
                        for remaining in data {
                            remaining.recycle(device, pool);
                        }
                        return Err(error.into());
                    }
                }
            }
            if top_input.is_none() {
                preparation_ns = elapsed(preparation);
                return Ok(());
            }
            let tlas_started = std::time::Instant::now();
            let mut resolved = Vec::new();
            resolved.try_reserve_exact(instances.len())?;
            for (index, instance) in instances.iter().enumerate() {
                let id = match instance.geometry {
                    crate::BatchBlas::Existing(blas) => blas,
                    crate::BatchBlas::New(index) => blases[index],
                };
                let Data::Blas(geometry) = life.resource(id.key)? else {
                    return Err(Error::InvalidArgument("BLAS 标识类型无效"));
                };
                resolved.push((
                    id,
                    ResolvedInstance {
                        geometry,
                        transform: instance.transform,
                        custom_index: instance.custom_index,
                        mask: masks.map_or(0xff, |m| m[index]),
                    },
                ));
            }
            let (data, buffers) = AccelerationData::record_instances_raw(
                device,
                &resolved,
                pool,
                instance_address,
                command,
            )?;
            drop(resolved);
            tlas_buffers = buffers;
            match life.insert_resource(Data::Acceleration(data)) {
                Ok(key) => top = Some(crate::AccelerationStructure { key }),
                Err((error, data)) => {
                    data.recycle(device, pool);
                    return Err(error.into());
                }
            }
            tlas_prepare_ns = elapsed(tlas_started);
            preparation_ns = elapsed(preparation);
            Ok(())
        };
        let mut raw = None;
        let result = if queued {
            terrain_flight::Recording::record(device, encode).map(|recorded| raw = Some(recorded))
        } else {
            immediate(device, encode)
        };
        let result = if let Some(raw) = raw {
            let buffers = [blas_buffers, tlas_buffers];
            self.submit_terrain(raw, buffers, |op| {
                if let Some(top) = top {
                    op.keepalive(top.into())?;
                }
                for &blas in &blases {
                    op.keepalive(blas.into())?;
                }
                if let Some(buffer) = instance_buffer {
                    op.access(buffer.into())?;
                }
                if let crate::acceleration::Sources::Quads(v)
                | crate::acceleration::Sources::Uploaded(v, _) = inputs
                {
                    for b in v
                        .iter()
                        .flat_map(|v| v.output.columns())
                        .filter_map(|(b, _, _)| b)
                    {
                        op.access(b.into())?;
                    }
                }
                if let crate::acceleration::Sources::Uploaded(_, staged) = inputs {
                    for s in staged {
                        op.access(s.buffer.into())?;
                    }
                }
                Ok(())
            })
        } else {
            blas_buffers.recycle(&self.device, &mut self.as_pool);
            tlas_buffers.recycle(&self.device, &mut self.as_pool);
            result
        };
        timing.submit_wait_ns = elapsed(started).saturating_sub(preparation_ns);
        if let Err(error) = result {
            if let Some(top) = top {
                self.discard(top.key);
            }
            for blas in blases {
                self.discard(blas.key);
            }
            return Err(error);
        }
        timing.pool_idle_bytes = self.as_pool.stats().idle_bytes;
        timing.total_ns = elapsed(started);
        Ok((
            crate::TriangleBlasBatch { blases, timing },
            top,
            tlas_prepare_ns,
        ))
    }

    pub(crate) fn record_acceleration_instances(
        &mut self,
        instances: &[crate::TriangleInstance],
    ) -> Result<crate::interop::PreparedTlas, Error> {
        self.record_instances_from(instances, None)
    }
    pub(crate) fn instance_payload(
        &self,
        instances: &[crate::TriangleInstance],
    ) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(
            instances
                .len()
                .checked_mul(64)
                .ok_or(Error::InvalidArgument("TLAS 字节数溢出"))?,
        )?;
        for i in instances {
            self.active(i.geometry.key)?;
            let geometry = self.blas(i.geometry)?;
            bytes.extend_from_slice(&crate::acceleration::instance_bytes(&ResolvedInstance {
                geometry,
                transform: i.transform,
                custom_index: i.custom_index,
                mask: 0xff,
            })?);
        }
        Ok(bytes)
    }
    pub(crate) fn record_instances_from(
        &mut self,
        instances: &[crate::TriangleInstance],
        input: Option<Buffer>,
    ) -> Result<crate::interop::PreparedTlas, Error> {
        let address = input
            .map(|b| -> Result<u64, Error> { Ok(b.data(self)?.address(&self.device)) })
            .transpose()?;
        let mut resolved = Vec::new();
        resolved.try_reserve_exact(instances.len())?;
        let mut roots = Vec::new();
        roots.try_reserve_exact(1)?;
        for instance in instances {
            self.active(instance.geometry.key)?;
            let Data::Blas(geometry) = self.life.resource(instance.geometry.key)? else {
                return Err(Error::InvalidArgument("BLAS 标识类型无效"));
            };
            resolved.push((
                instance.geometry,
                ResolvedInstance {
                    geometry,
                    transform: instance.transform,
                    custom_index: instance.custom_index,
                    mask: 0xff,
                },
            ));
        }
        let (data, raw, buffers) = AccelerationData::record_instances(
            &self.device,
            &resolved,
            &mut self.as_pool,
            address,
        )?;
        drop(resolved);
        let key = match self.insert(Data::Acceleration(data)) {
            Ok(key) => key,
            Err(error) => {
                buffers.recycle(&self.device, &mut self.as_pool);
                return Err(error);
            }
        };
        roots.push(key);
        let record = match self.insert_build_record(Record::new(
            input.into_iter().map(|b| Op::Access(b.into())).collect(),
            input.into_iter().map(|b| b.key).collect(),
            roots,
            buffers,
        )) {
            Ok(record) => record,
            Err(error) => {
                self.discard(key);
                return Err(error);
            }
        };
        Ok(crate::interop::PreparedTlas {
            acceleration: crate::AccelerationStructure { key },
            commands: crate::interop::CommandBuffer::recorded(raw, record),
        })
    }
    /// 同步构建 TLAS；同设备 BLAS/OMM 依赖由本 owner 保存。
    pub fn acceleration_instances(
        &mut self,
        instances: &[crate::TriangleInstance],
    ) -> Result<crate::AccelerationStructure, Error> {
        let data = AccelerationData::native_instances(&self.device, instances, |id| {
            self.active(id.key)?;
            self.producer_submitted(id.key)?;
            self.blas(id)
        })?;
        Ok(crate::AccelerationStructure {
            key: self.insert(Data::Acceleration(data))?,
        })
    }
    /// 同步构建精确二值 OMM；支持原生与 wgpu 设备，返回后释放输入借用和暂存。
    /// 逻辑设备须启用 KHR OMM。Context 管理其生命周期，BLAS 自动保留依赖；
    /// 尚未被 BLAS 引用的表须作为 collect 的根保活。失败保留原有资源。
    /// # Errors
    /// 不支持、输入或设备失败返回来源；无法确认完成时隔离在途资源。
    pub fn opacity_micromap(
        &mut self,
        maps: &[crate::OpacityMap<'_>],
    ) -> Result<crate::OpacityMicromap, Error> {
        let data = MicromapData::new(&self.device, maps)?;
        let count = data.map_count();
        Ok(crate::OpacityMicromap {
            key: self.insert(Data::Micromap(data))?,
            count,
        })
    }
    pub(crate) fn triangle_blas_with_opacity(
        &mut self,
        triangles: &[[[f32; 3]; 3]],
        micromap: crate::OpacityMicromap,
        indices: &[u32],
    ) -> Result<crate::TriangleBlas, Error> {
        self.active(micromap.key)?;
        let Data::Micromap(omm) = self.get(micromap.key)? else {
            return Err(Error::InvalidArgument("OMM 标识类型无效"));
        };
        let data = BlasData::with_opacity(&self.device, triangles, (omm, micromap), indices)?;
        Ok(crate::TriangleBlas {
            key: self.insert(Data::Blas(data))?,
        })
    }
    pub(super) fn blas(&self, id: crate::TriangleBlas) -> Result<&BlasData, Error> {
        match self.get(id.key)? {
            Data::Blas(data) => Ok(data),
            _ => Err(Error::InvalidArgument("BLAS 标识类型无效")),
        }
    }
    pub(super) fn acceleration(
        &self,
        id: crate::AccelerationStructure,
    ) -> Result<&AccelerationData, Error> {
        match self.get(id.key)? {
            Data::Acceleration(data) => Ok(data),
            _ => Err(Error::InvalidArgument("TLAS 标识类型无效")),
        }
    }
}
