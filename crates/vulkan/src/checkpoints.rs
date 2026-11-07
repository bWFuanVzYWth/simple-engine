//! 可选 NV 异步进度诊断；标记不加入 fence、提交或内存屏障。
//!
//! marker 是不解引用的递增整数；固定容量日志只解释近期 marker，淘汰项保持未知。
//! 查询遵循 [DEVICE_LOST 前提](https://docs.vulkan.org/refpages/latest/refpages/source/vkGetQueueCheckpointDataNV.html)，
//! 返回的是驱动阶段最后经过的标记，不是某条命令有罪或整帧完成的证明。

use crate::{Error, native::Context, vk};
use ash::vk::Handle;
use std::{collections::VecDeque, sync::Mutex};

const RETAINED: usize = 512;

/// 已录入命令流的诊断标记；录入不代表提交或执行完成。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Checkpoint {
    /// 当前 Context 内不复用的 opaque marker。
    pub marker: usize,
    /// 应用提供的帧或阶段序号。
    pub scope: u64,
    /// trace、dispatch 或应用边界及其 begin/end。
    pub label: &'static str,
    /// trace/dispatch 的 VkPipeline 原始值；应用边界为零。
    pub pipeline: u64,
    /// trace invocation 尺寸、dispatch workgroup 数量；应用边界为零。
    pub extent: [u32; 3],
}

/// 驱动报告的单个流水线阶段进度。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointProgress {
    /// VkPipelineStageFlagBits 原始位；由驱动定义哪些阶段有进度。
    pub stage: u32,
    /// 驱动返回的 opaque marker；绝不作为地址解引用。
    pub marker: usize,
    /// marker 仍在近期日志中时的说明；未知或已淘汰时为 None。
    pub checkpoint: Option<Checkpoint>,
}

/// 设备丢失后的异步进度快照；不恢复设备或改变在途资源所有权。
#[derive(Debug)]
pub struct CheckpointReport {
    /// 驱动返回的阶段进度，允许为空。
    pub progress: Vec<CheckpointProgress>,
    /// 最后最多 512 个已录入标记，按 CPU 录制顺序；不表示已执行。
    pub recorded: Vec<Checkpoint>,
}

struct Journal {
    next: usize,
    scope: u64,
    records: VecDeque<Checkpoint>,
}

impl Journal {
    fn new() -> Result<Self, Error> {
        let mut records = VecDeque::new();
        records.try_reserve_exact(RETAINED)?;
        Ok(Self {
            next: 1,
            scope: 0,
            records,
        })
    }

    fn push(
        &mut self,
        label: &'static str,
        pipeline: u64,
        extent: [u32; 3],
    ) -> Result<usize, Error> {
        let marker = self.next;
        self.next = marker
            .checked_add(1)
            .ok_or(Error::InvalidState("GPU 检查点标识耗尽"))?;
        if self.records.len() == RETAINED {
            self.records.pop_front();
        }
        self.records.push_back(Checkpoint {
            marker,
            scope: self.scope,
            label,
            pipeline,
            extent,
        });
        Ok(marker)
    }

    fn resolve(&self, marker: usize) -> Option<Checkpoint> {
        self.records
            .iter()
            .find(|record| record.marker == marker)
            .copied()
    }
}

pub(super) struct Checkpoints {
    loader: ash::nv::device_diagnostic_checkpoints::Device,
    // 临界区仅修改标量／有界 Copy 日志，分配与序号失败返回 Err；不调用驱动或外部回调。
    journal: Mutex<Journal>,
}

impl Context {
    /// 启用当前 Context 的 NV 异步检查点；重复调用保留同一 marker 序列。
    /// 正常默认路径不创建日志或调用 NV 检查点。此模式仅用于诊断，不用于性能验收。
    /// # Errors
    /// 设备已失效或固定容量日志分配失败时返回错误。
    /// # Safety
    /// 宿主必须已在此逻辑设备启用 `VK_NV_device_diagnostic_checkpoints`；仅物理支持不够。
    pub unsafe fn enable_diagnostic_checkpoints(&mut self) -> Result<(), Error> {
        self.device.ensure_active()?;
        if self.checkpoints.is_none() {
            self.checkpoints = Some(Checkpoints {
                loader: ash::nv::device_diagnostic_checkpoints::Device::load(
                    &self.device.physical.instance.raw,
                    &self.device.raw,
                ),
                journal: Mutex::new(Journal::new()?),
            });
        }
        Ok(())
    }

    /// 为后续诊断标记关联应用序号；未启用时无操作，不提交 GPU 工作。
    pub fn set_diagnostic_scope(&mut self, scope: u64) {
        if let Some(checkpoints) = &mut self.checkpoints {
            checkpoints
                .journal
                .get_mut()
                .expect("私有诊断临界区无 panic 或外部回调")
                .scope = scope;
        }
    }

    pub(super) fn checkpoint(
        &self,
        command: vk::CommandBuffer,
        label: &'static str,
        pipeline: u64,
        extent: [u32; 3],
    ) -> Result<(), Error> {
        if let Some(checkpoints) = &self.checkpoints {
            let marker = checkpoints
                .journal
                .lock()
                .expect("私有诊断临界区无 panic 或外部回调")
                .push(label, pipeline, extent)?;
            // SAFETY: 创建时确认扩展已启用；调用方维护独占录制。marker 是规范允许的 opaque 值，不解引用。
            unsafe {
                checkpoints
                    .loader
                    .cmd_set_checkpoint(command, std::ptr::without_provenance(marker))
            };
        }
        Ok(())
    }

    /// 在外部 SDK 等命令前后录入诊断边界；未启用时无操作。
    /// # Errors
    /// command 为空或标记耗尽时返回错误；失败后不得提交部分录制的 command。
    /// # Safety
    /// command 属于本设备及队列族，独占处于录制状态，不在 video coding 或 suspended render pass 内。
    pub unsafe fn diagnostic_checkpoint(
        &self,
        raw_command: u64,
        label: &'static str,
    ) -> Result<(), Error> {
        if raw_command == 0 {
            return Err(Error::InvalidArgument("GPU 检查点 command 为空"));
        }
        self.checkpoint(vk::CommandBuffer::from_raw(raw_command), label, 0, [0; 3])
    }

    /// 查询已丢失设备的队列进度；未启用时返回 None，不调用驱动。
    /// 不等待 GPU，不标记任何 Flight 完成，也不销毁资源。
    /// # Errors
    /// 诊断输出分配失败时返回错误。
    /// # Safety
    /// 宿主已收到此设备的 `VK_ERROR_DEVICE_LOST`，设备和本 Context 的 queue 句柄仍有效，且独占 queue。
    /// 不得以普通超时或应用异常代替设备丢失前提。
    pub unsafe fn checkpoints_after_device_loss(&self) -> Result<Option<CheckpointReport>, Error> {
        let Some(checkpoints) = &self.checkpoints else {
            return Ok(None);
        };
        let queue = self.device.queue;
        let function = checkpoints.loader.fp().get_queue_checkpoint_data_nv;
        let mut count = 0;
        // SAFETY: 调用者已确认 DEVICE_LOST 且保活 queue；第一次仅查询所需容量。
        unsafe { function(queue, &mut count, std::ptr::null_mut()) };
        let mut data = Vec::new();
        data.try_reserve_exact(count as usize)?;
        data.resize(count as usize, vk::CheckpointDataNV::default());
        if count != 0 {
            // SAFETY: 每个输出结构初始化了 sType；驱动最多写入传入 count 个元素。
            // 直接用绑定兼容两次查询间数量减少，避免 ash convenience wrapper 的等长 assert。
            unsafe { function(queue, &mut count, data.as_mut_ptr()) };
            data.truncate(count as usize);
        }
        let journal = checkpoints
            .journal
            .lock()
            .expect("私有诊断临界区无 panic 或外部回调");
        let mut report = CheckpointReport {
            progress: Vec::new(),
            recorded: Vec::new(),
        };
        report.progress.try_reserve_exact(data.len())?;
        report.recorded.try_reserve_exact(journal.records.len())?;
        report.recorded.extend(journal.records.iter().copied());
        for value in data {
            let marker = value.p_checkpoint_marker.addr();
            report.progress.push(CheckpointProgress {
                stage: value.stage.as_raw(),
                marker,
                checkpoint: journal.resolve(marker),
            });
        }
        Ok(Some(report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retired_marker_cannot_alias_later_record() {
        let mut journal = Journal::new().unwrap();
        let old = journal.push("first", 7, [1; 3]).unwrap();
        journal.scope = 42;
        for _ in 0..RETAINED {
            journal.push("later", 8, [2; 3]).unwrap();
        }
        assert_eq!(journal.resolve(old), None);
        let newest = journal.records.back().unwrap();
        assert_eq!(newest.scope, 42);
        assert_eq!(journal.resolve(newest.marker), Some(*newest));
        assert_eq!(journal.records.len(), RETAINED);
    }

    #[test]
    fn exhausted_marker_does_not_modify_existing_evidence() {
        let mut journal = Journal::new().unwrap();
        let first = journal.push("first", 7, [1; 3]).unwrap();
        journal.next = usize::MAX;
        assert!(journal.push("overflow", 8, [2; 3]).is_err());
        assert_eq!(journal.records.len(), 1);
        assert_eq!(journal.resolve(first).unwrap().label, "first");
    }
}
