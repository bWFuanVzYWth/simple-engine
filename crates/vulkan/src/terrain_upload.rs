//! 一批地形包的跨帧上传游标。完整域可大于单帧预算，未完成时继续使用已发布场景。
use crate::{
    Error, QuadBlasInput,
    native::{Buffer, Context, Resource},
};

// 上传字节与展开/AS 的工作量独立。单个超大 BLAS 必须完整交给驱动；其余区域按批排队。
const BUILD_QUADS: usize = 128 * 1024;
const BUILD_REGIONS: usize = 32;
const DEFAULT_LIMIT: usize = 64 * 1024 * 1024;
/// 每个渲染所有者独占一份。默认额度 64 MiB，同批尽量用满，最后一帧只复制实际剩余字节。
/// 被后续编辑替代的包只有在已写入前缀逐字节相同时才续传。旧缓冲通过 GPU 资源记录退役。
pub struct TerrainUploads {
    limit: usize,
    items: Vec<Packet>,
    transferred: usize,
    built_quads: usize,
    side_bytes: usize,
    prepared: Option<Prepared>,
    instances: Option<Item>,
}
struct Prepared {
    stamps: Vec<(crate::QuadOutput, bool, Option<crate::OpacityMicromap>, u64)>,
    blases: Vec<crate::TriangleBlas>,
    timing: crate::BuildTiming,
}
struct Packet {
    buffer: Option<Buffer>,
    serial: u64,
    len: usize,
    uploaded: usize,
    opacity: Vec<u32>,
}
struct Item {
    buffer: Buffer,
    prefix: Vec<u8>,
    len: usize,
}
impl Default for TerrainUploads {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            items: Vec::new(),
            transferred: 0,
            built_quads: 0,
            side_bytes: 0,
            prepared: None,
            instances: None,
        }
    }
}
/// 已按生产流校验的设备布局，不向应用暴露可替换的任意设备地址。
#[derive(Clone, Copy)]
pub(crate) struct Uploaded {
    pub buffer: Buffer,
}
impl TerrainUploads {
    /// 设置每帧字节上限，至少 4B 并按 word 对齐。
    pub fn set_limit(&mut self, bytes: usize) -> Result<(), Error> {
        if bytes < 4 || !bytes.is_multiple_of(4) {
            return Err(Error::InvalidArgument("地形预算须为非零的4B整数倍"));
        }
        self.limit = bytes;
        Ok(())
    }
    /// 每帧允许的 PCIe 地形数据量。
    pub const fn limit(&self) -> usize {
        self.limit
    }
    /// 本帧实际传输的几何、身份、OMM、实例、灯表和属性补丁字节。
    pub const fn transferred(&self) -> usize {
        self.transferred + self.side_bytes
    }
    /// 本次实际安排展开与 BLAS 构建的 quad 量，与已上传但尚未开工的几何分别计量。
    pub const fn built_quads(&self) -> usize {
        self.built_quads
    }
    /// 从当前帧预算中扣除已推进的目录、灯表及属性补丁。
    pub const fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.transferred())
    }
    /// 在实际帧的入口重置本帧计数，跨帧候选继续归所有者所有。
    pub fn begin_frame(&mut self) {
        self.transferred = 0;
        self.built_quads = 0;
        self.side_bytes = 0;
    }
    /// 尚有跨帧候选，渲染所有者应继续安排准备。
    pub const fn pending(&self) -> bool {
        !self.items.is_empty() || self.instances.is_some() || self.prepared.is_some()
    }
    /// 在途上传的设备缓冲须作为 collect 的当前发布根。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.items
            .iter()
            .filter_map(|p| p.buffer.map(Resource::from))
            .chain(self.instances.iter().map(|p| Resource::from(p.buffer)))
            .chain(
                self.prepared
                    .iter()
                    .flat_map(|p| p.blases.iter().copied().map(Resource::from)),
            )
            .chain(self.prepared.iter().flat_map(|p| {
                p.stamps.iter().flat_map(|s| {
                    s.0.columns()
                        .into_iter()
                        .filter_map(|(b, _, _)| b.map(Resource::from))
                })
            }))
    }
    /// 丢弃尚未发布的候选，已提交的 GPU 命令仍持有其根。
    pub fn clear(&mut self) {
        self.items.clear();
        self.instances = None;
        self.prepared = None;
    }
    fn trim_builds(&mut self, count: usize) {
        if let Some(p) = &mut self.prepared
            && p.blases.len() > count
        {
            p.blases.truncate(count);
            p.stamps.truncate(count);
            self.instances = None;
            // 被替代的批次不计入新事务的工作统计。
            p.timing = Default::default();
            p.timing.blas_count = count;
        }
    }
    pub(crate) fn remember(
        &mut self,
        inputs: &[QuadBlasInput<'_>],
        batch: &crate::TriangleBlasBatch,
    ) -> Result<(), Error> {
        let p = self.prepared.get_or_insert_with(|| Prepared {
            stamps: Vec::new(),
            blases: Vec::new(),
            timing: Default::default(),
        });
        p.stamps.try_reserve(inputs.len())?;
        p.blases.try_reserve(batch.blases.len())?;
        let first = p.blases.len();
        for packet in &mut self.items[first..first + inputs.len()] {
            packet.buffer = None;
        }
        self.built_quads += inputs
            .iter()
            .map(|i| i.quads.positions().headers().len())
            .sum::<usize>();
        p.stamps.extend(inputs.iter().map(stamp));
        p.blases.extend_from_slice(&batch.blases);
        p.timing.merge(batch.timing);
        Ok(())
    }
    pub(crate) fn ready(
        &mut self,
        count: usize,
        tail: usize,
    ) -> Result<Option<crate::TriangleBlasBatch>, Error> {
        let n = self.prepared.as_ref().map_or(0, |p| p.blases.len());
        if n != count || tail <= self.limit && tail > self.remaining() {
            return Ok(None);
        }
        if tail <= self.limit {
            self.transferred += tail;
        }
        let mut blases = Vec::new();
        blases.try_reserve_exact(n)?;
        let mut timing = Default::default();
        if let Some(p) = &self.prepared {
            blases.extend_from_slice(&p.blases);
            timing = p.timing;
        }
        Ok(Some(crate::TriangleBlasBatch { blases, timing }))
    }
    pub(crate) fn instance_buffer(
        &mut self,
        ctx: &mut Context,
        data: &[u8],
    ) -> Result<Option<Buffer>, Error> {
        let n = data.len();
        if self
            .instances
            .as_ref()
            .is_none_or(|v| v.len != n || !data.starts_with(&v.prefix))
        {
            self.instances = Some(Item {
                buffer: Buffer::mapped_device_local(ctx, n as u64)?,
                prefix: Vec::new(),
                len: n,
            });
        }
        let left = self.remaining();
        let item = self.instances.as_mut().expect("实例暂存已建立");
        let from = item.prefix.len();
        let end = n.min(from.saturating_add(left));
        if from != end {
            item.prefix.try_reserve(end - from)?;
            item.buffer.write(ctx, from as u64, &data[from..end])?;
            item.prefix.extend_from_slice(&data[from..end]);
            self.transferred += end - from;
        }
        Ok((end == n).then_some(item.buffer))
    }
    pub(crate) fn advance(
        &mut self,
        ctx: &mut Context,
        inputs: &[QuadBlasInput<'_>],
        remaining: usize,
        tail: usize,
    ) -> Result<Progress, Error> {
        let mut quota = remaining.min(self.remaining()) & !3;
        let size = |i: &QuadBlasInput<'_>| parts(i).iter().map(|b| b.len()).sum::<usize>();
        let total = inputs.iter().try_fold(0usize, |n, i| {
            n.checked_add(size(i))
                .ok_or(Error::InvalidArgument("地形输入总量溢出"))
        })?;
        let split = inputs.len() > 1
            && (inputs.len() > BUILD_REGIONS
                || inputs
                    .iter()
                    .map(|i| i.quads.positions().headers().len())
                    .sum::<usize>()
                    > BUILD_QUADS);
        if self.prepared.is_none()
            && self.items.is_empty()
            && !split
            && total.saturating_add(tail) <= quota
        {
            self.transferred += total + tail;
            self.built_quads += inputs
                .iter()
                .map(|i| i.quads.positions().headers().len())
                .sum::<usize>();
            return Ok(Progress::Direct);
        }
        if let Some(p) = &self.prepared {
            let valid = p
                .stamps
                .iter()
                .copied()
                .zip(inputs.iter().map(stamp))
                .take_while(|(a, b)| a == b)
                .count();
            self.trim_builds(valid);
        }
        self.items.truncate(inputs.len());
        let mut ready = 0;
        for (index, input) in inputs.iter().enumerate() {
            let data = parts(input);
            let len = data.iter().map(|v| v.len()).sum::<usize>();
            if len == 0 {
                return Err(Error::InvalidArgument("空地形包不能建BLAS"));
            }
            let opacity = input.opacity.map_or(&[][..], |(_, v)| v);
            let same = self.items.get(index).is_some_and(|p| {
                p.len == len
                    && p.serial == input.quads.serial()
                    && p.opacity == opacity
                    && (p.buffer.is_some()
                        || index < self.prepared.as_ref().map_or(0, |b| b.blases.len()))
            });
            if !same {
                self.items.truncate(index);
                self.trim_builds(index);
                self.instances = None;
                if quota == 0 {
                    break;
                }
                let mut indices = Vec::new();
                indices.try_reserve_exact(opacity.len())?;
                indices.extend_from_slice(opacity);
                self.items.try_reserve(1)?;
                self.items.push(Packet {
                    buffer: Some(Buffer::mapped_device_local(ctx, len as u64)?),
                    serial: input.quads.serial(),
                    len,
                    uploaded: 0,
                    opacity: indices,
                });
            }
            let item = &mut self.items[index];
            let start = item.uploaded;
            let write = (len - start).min(quota);
            if write != 0 {
                let end = start + write;
                let mut at = 0;
                let mut ranges = [(0usize, &[][..]); 5];
                let mut n = 0;
                for part in data {
                    let first = start.max(at);
                    let last = end.min(at + part.len());
                    if first < last {
                        ranges[n] = (first, &part[first - at..last - at]);
                        n += 1;
                    }
                    at += part.len();
                }
                item.buffer
                    .expect("未完成的包保有上传目标")
                    .write_ranges(ctx, &ranges[..n])?;
                item.uploaded = end;
                quota -= write;
                self.transferred += write;
            }
            if item.uploaded != len {
                break;
            }
            ready += 1;
        }
        let begin = self.prepared.as_ref().map_or(0, |p| p.blases.len());
        let mut work = 0usize;
        let mut end = begin;
        for input in &inputs[begin..ready.max(begin)] {
            let n = input.quads.positions().headers().len();
            if end > begin && (work.saturating_add(n) > BUILD_QUADS || end - begin >= BUILD_REGIONS)
            {
                break;
            }
            work += n;
            end += 1;
        }
        let mut buffers = Vec::new();
        buffers.try_reserve_exact(end - begin)?;
        buffers.extend(self.items[begin..end].iter().map(|p| Uploaded {
            buffer: p.buffer.expect("未提交的完整区间仍有上传缓冲"),
        }));
        Ok(Progress::Staged {
            start: begin,
            buffers,
        })
    }
}
/// 与一批冻结的目录、属性及灯列一起移动的上传游标。每批创建新游标，数据源由应用事务独占。
/// 所有列在该游标存活期内须保持内容与顺序；跨帧只保留设备目标和字节偏移。
#[derive(Default)]
pub struct ColumnUpload {
    items: Vec<(Buffer, usize)>,
}
impl ColumnUpload {
    /// 候选尚未提交时需保活的目标。
    pub fn roots(&self) -> impl Iterator<Item = Resource> + '_ {
        self.items.iter().map(|p| p.0.into())
    }
    /// 消费与几何共享的帧额度。只读取本帧新后缀，全部完成时返回设备列。
    pub fn stage(
        &mut self,
        ctx: &mut Context,
        budget: &mut TerrainUploads,
        columns: &[&[u8]],
    ) -> Result<Option<Vec<Buffer>>, Error> {
        let mut result = Vec::new();
        result.try_reserve_exact(columns.len())?;
        if self.items.len() > columns.len() {
            return Err(Error::InvalidArgument("冻结列数量改变"));
        }
        for (i, &part) in columns.iter().enumerate() {
            if part.is_empty() || !part.len().is_multiple_of(4) {
                return Err(Error::InvalidArgument("地形附属列大小无效"));
            }
            if self.items.len() == i {
                if budget.remaining() == 0 {
                    return Ok(None);
                }
                self.items.try_reserve(1)?;
                self.items
                    .push((Buffer::mapped_device_local(ctx, part.len() as u64)?, 0));
            }
            let (buffer, at) = &mut self.items[i];
            if buffer.len() != part.len() as u64 {
                return Err(Error::InvalidArgument("冻结列大小改变"));
            }
            let end = part.len().min(at.saturating_add(budget.remaining()));
            if end > *at {
                buffer.write(ctx, *at as u64, &part[*at..end])?;
                budget.side_bytes += end - *at;
                *at = end;
            }
            if *at != part.len() {
                return Ok(None);
            }
            result.push(*buffer);
        }
        Ok(Some(result))
    }
}

fn stamp(v: &QuadBlasInput<'_>) -> (crate::QuadOutput, bool, Option<crate::OpacityMicromap>, u64) {
    (
        v.output,
        v.opaque,
        v.opacity.map(|(m, _)| m),
        v.quads.serial(),
    )
}
pub(crate) enum Progress {
    Direct,
    Staged {
        start: usize,
        buffers: Vec<Uploaded>,
    },
}
fn parts<'a>(i: &QuadBlasInput<'a>) -> [&'a [u8]; 5] {
    use bytemuck::cast_slice;
    let q = i.quads.positions();
    [
        cast_slice(q.headers()),
        cast_slice(q.surfaces()),
        cast_slice(q.payload()),
        if i.output.identities.is_some() && !i.quads.consecutive() {
            cast_slice(i.quads.explicit_tokens())
        } else {
            &[]
        },
        i.opacity.map_or(&[], |(_, v)| cast_slice(v)),
    ]
}
