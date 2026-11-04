//! 由 Context 保活的诊断 timestamp；读取不使用 WAIT，也不提交命令。

use super::{Context, Data};
use crate::{Device, Error, owner::Destroy, vk};

#[doc(inline)]
pub use crate::handles::TimestampQueries;

impl TimestampQueries {
    /// 创建非空 pool；主队列不支持 timestamp 时返回 Unsupported。
    pub fn new(context: &mut Context, count: u32) -> Result<Self, Error> {
        context.device.ensure_active()?;
        if count == 0 {
            return Err(Error::InvalidArgument("timestamp 数量不能为零"));
        }
        let device = &context.device;
        let bits =
            device.physical.queue_families()[device.queue_family as usize].timestamp_valid_bits;
        if bits == 0 {
            return Err(Error::Unsupported("主队列不支持 timestamp"));
        }
        let raw = unsafe {
            device.raw.create_query_pool(
                &vk::QueryPoolCreateInfo::default()
                    .query_type(vk::QueryType::TIMESTAMP)
                    .query_count(count),
                None,
            )?
        };
        let period = f64::from(device.physical.properties().limits.timestamp_period);
        let key = context.insert(Data::Timestamps(QueryData {
            raw,
            count,
            bits,
            period,
            initialized: false,
        }))?;
        Ok(Self { key, count })
    }

    /// 固定查询槽数。
    pub const fn len(self) -> u32 {
        self.count
    }

    /// 构造拒绝空 pool。
    pub const fn is_empty(self) -> bool {
        false
    }

    pub(super) fn data(self, context: &Context) -> Result<&QueryData, Error> {
        match context.get(self.key)? {
            Data::Timestamps(data) => Ok(data),
            _ => Err(Error::InvalidArgument("timestamp 标识类型无效")),
        }
    }

    /// 读取全部原始 tick；未写入或尚未 ready 返回 None，不等待 GPU。
    /// 有待执行或在途使用时拒绝，宿主须先收取真实完成通知；返回值不延长 pool 寿命。
    pub fn read(self, context: &Context) -> Result<Option<Vec<u64>>, Error> {
        context.active(self.key)?;
        context.resource_idle(self.key)?;
        let data = self.data(context)?;
        if !data.initialized {
            return Ok(None);
        }
        let mut values = vec![[0u64; 2]; self.count as usize];
        let result = unsafe {
            context.device.raw.get_query_pool_results(
                data.raw,
                0,
                &mut values,
                vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WITH_AVAILABILITY,
            )
        };
        match result {
            Ok(()) if values.iter().all(|value| value[1] != 0) => {
                Ok(Some(values.into_iter().map(|value| value[0]).collect()))
            }
            Ok(()) | Err(vk::Result::NOT_READY) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// 按设备 timestampPeriod 和有效位宽计算两个 tick 的 ns；支持一次计数回绕。
    /// 跨度必须短于设备计数器完整周期；这是 GPU 调度跨度，非独占 shader 执行成本。
    pub fn elapsed_ns(self, context: &Context, start: u64, end: u64) -> Result<u64, Error> {
        let data = self.data(context)?;
        Ok(elapsed_ns(start, end, data.bits, data.period))
    }
}

pub(crate) struct QueryData {
    pub(super) raw: vk::QueryPool,
    pub(super) count: u32,
    pub(super) initialized: bool,
    bits: u32,
    period: f64,
}

impl Destroy for QueryData {
    fn destroy(&mut self, device: &Device) {
        unsafe {
            device.raw.destroy_query_pool(self.raw, None);
        }
    }
}

fn elapsed_ns(start: u64, end: u64, bits: u32, period: f64) -> u64 {
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    ((end.wrapping_sub(start) & mask) as f64 * period).round() as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_wrap_uses_device_width() {
        assert_eq!(super::elapsed_ns(250, 5, 8, 2.0), 22);
    }

    #[test]
    #[ignore = "需要真实Vulkan timestamp队列；主任务串行执行"]
    fn timestamp_queries_follow_record_completion() -> Result<(), crate::Error> {
        use crate::{interop::WgpuContext, native::Commands};
        let mut context = pollster::block_on(WgpuContext::compute())?;
        let queries = super::TimestampQueries::new(context.native(), 2)?;
        assert!(queries.read(context.native_ref())?.is_none());
        let mut commands = Commands::new(context.native_ref())?;
        commands.reset_timestamps(queries)?;
        commands.write_timestamp(queries, 0)?;
        commands.barrier();
        commands.write_timestamp(queries, 1)?;
        let commands = commands.finish(context.native())?;
        assert!(queries.read(context.native_ref()).is_err());
        let command = context.record_native(commands)?;
        let submission = context.submit([command])?;
        assert!(queries.read(context.native_ref()).is_err());
        context.device().poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: None,
        })?;
        context.completed_serial();
        let values = queries
            .read(context.native_ref())?
            .expect("真实完成后查询可读");
        assert_eq!(values.len(), 2);
        queries.elapsed_ns(context.native_ref(), values[0], values[1])?;
        Ok(())
    }
}
