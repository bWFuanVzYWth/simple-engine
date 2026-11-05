use crate::{Error, khr, native::Context, vk};

/// 驱动定义的单个统计值；单位和语义见对应描述，不等同于 SPIR-V 字数或栈需求。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatisticValue {
    /// Vulkan BOOL32。
    Bool(bool),
    /// 有符号 64 位整数。
    I64(i64),
    /// 无符号 64 位整数。
    U64(u64),
    /// 64 位浮点数。
    F64(f64),
}

/// 一个可执行单元公开的驱动统计。
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutableStatistic {
    /// 驱动提供的名称。
    pub name: String,
    /// 驱动提供的含义和单位。
    pub description: String,
    /// 按 Vulkan 声明格式解码的值。
    pub value: StatisticValue,
}

/// 链接后可执行单元及其统计；驱动可能合并多个着色阶段。
#[derive(Clone, Debug, PartialEq)]
pub struct ExecutableStatistics {
    /// 驱动提供的名称。
    pub name: String,
    /// 驱动提供的描述。
    pub description: String,
    /// 涉及的 Vulkan ShaderStageFlags 原始位。
    pub stages: u32,
    /// 驱动报告的 subgroup 规模。
    pub subgroup_size: u32,
    /// 可能为空；没有公开值不表示没有资源使用。
    pub statistics: Vec<ExecutableStatistic>,
}

fn text(value: &[std::ffi::c_char]) -> String {
    let bytes: Vec<_> = value
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(super) fn query(
    context: &Context,
    pipeline: vk::Pipeline,
) -> Result<Vec<ExecutableStatistics>, Error> {
    context.device.ensure_active()?;
    if !context.device.pipeline_statistics {
        return Err(Error::Unsupported("未启用 pipelineExecutableInfo"));
    }
    let loader = khr::pipeline_executable_properties::Device::load(
        &context.device.physical.instance.raw,
        &context.device.raw,
    );
    // SAFETY: Context 独占有效设备与 pipeline；创建时已启用 feature 和捕获标志。
    // 同一不可变 pipeline 的统计数量稳定；输出结构先初始化，union 仅读取 format 指定成员。
    unsafe {
        let info = vk::PipelineInfoKHR::default().pipeline(pipeline);
        let mut properties = vec![
            vk::PipelineExecutablePropertiesKHR::default();
            loader.get_pipeline_executable_properties_len(&info)?
        ];
        loader.get_pipeline_executable_properties(&info, &mut properties)?;
        let mut result = Vec::with_capacity(properties.len());
        for (index, property) in properties.iter().enumerate() {
            let info = vk::PipelineExecutableInfoKHR::default()
                .pipeline(pipeline)
                .executable_index(index as u32);
            let mut values = vec![
                vk::PipelineExecutableStatisticKHR::default();
                loader.get_pipeline_executable_statistics_len(&info)?
            ];
            loader.get_pipeline_executable_statistics(&info, &mut values)?;
            let mut statistics = Vec::with_capacity(values.len());
            for statistic in values {
                let value = match statistic.format {
                    vk::PipelineExecutableStatisticFormatKHR::BOOL32 => {
                        StatisticValue::Bool(statistic.value.b32 != 0)
                    }
                    vk::PipelineExecutableStatisticFormatKHR::INT64 => {
                        StatisticValue::I64(statistic.value.i64)
                    }
                    vk::PipelineExecutableStatisticFormatKHR::UINT64 => {
                        StatisticValue::U64(statistic.value.u64)
                    }
                    vk::PipelineExecutableStatisticFormatKHR::FLOAT64 => {
                        StatisticValue::F64(statistic.value.f64)
                    }
                    _ => {
                        return Err(Error::Unsupported(
                            "未知 pipeline executable statistic 格式",
                        ));
                    }
                };
                statistics.push(ExecutableStatistic {
                    name: text(&statistic.name),
                    description: text(&statistic.description),
                    value,
                });
            }
            result.push(ExecutableStatistics {
                name: text(&property.name),
                description: text(&property.description),
                stages: property.stages.as_raw(),
                subgroup_size: property.subgroup_size,
                statistics,
            });
        }
        Ok(result)
    }
}
