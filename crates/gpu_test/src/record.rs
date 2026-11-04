//! 自包含的 JSON 失败记录；优先重放完整输入，再用 seed、stream、index 定位与缩小。
//!
//! 浮点数据保存原始 `u32` bits，保留 NaN payload 与 signed zero。输入字段的解释和约束由
//! 用例的 schema 定义；加载只验证记录格式与随机协议，schema 匹配由用例负责。
//! 依赖 dispatch 位置或并发顺序的失败须另行保留现场，缩小输入不能改变原属性契约。

use crate::input::RNG_PROTOCOL;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Write},
    path::Path,
};

/// 自包含失败记录的格式版本。
pub const FAILURE_FORMAT: &str = "gpu-test-failure-v1";

/// 重放属性失败所需的输入、结果与构建身份。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    /// 属性用例的稳定名称。
    pub case: String,
    /// 输入布局与约束版本，由用例验证。
    pub schema: String,
    /// 完整 256-bit seed，按字节顺序保存。
    pub seed: [u8; 32],
    /// 属性专用的 ChaCha20 stream。
    pub stream: u64,
    /// 不受批次划分影响的绝对样本索引。
    pub index: u64,
    /// 完整输入字；浮点值使用原始 bits。
    pub input: Vec<u32>,
    /// 观察到的 GPU 输出字，编码规则同输入。
    pub actual: Vec<u32>,
    /// 原失败设备的可读身份。
    pub device: String,
    /// 原失败驱动的版本描述。
    pub driver: String,
    /// 参与执行的 shader 构建指纹。
    pub shader_blake3: String,
}

/// 记录 IO、JSON 或协议不兼容；底层错误保留来源。
#[derive(Debug, thiserror::Error)]
pub enum RecordError {
    /// 文件打开、写入或 flush 失败。
    #[error("读写 GPU 测试记录失败：{0}")]
    Io(#[from] std::io::Error),
    /// 内容无法按 JSON 记录解码或编码。
    #[error("GPU 测试记录 JSON 错误：{0}")]
    Json(#[from] serde_json::Error),
    /// 记录采用未知格式，携带原格式名。
    #[error("不支持的失败记录格式：{0}")]
    Format(String),
    /// 记录采用未知随机协议，携带原协议名。
    #[error("不支持的属性输入协议：{0}")]
    Rng(String),
}

#[derive(Serialize, Deserialize)]
struct Record<T> {
    format: String,
    rng: String,
    #[serde(flatten)]
    failure: T,
}

impl Failure {
    /// 覆盖指定文件；父目录由调用方创建。浮点输入和结果以原始 u32 bits 保存。
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), RecordError> {
        let mut writer = BufWriter::new(File::create(path)?);
        serde_json::to_writer_pretty(
            &mut writer,
            &Record {
                format: FAILURE_FORMAT.to_owned(),
                rng: RNG_PROTOCOL.to_owned(),
                failure: self,
            },
        )?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        Ok(())
    }

    /// 读取并验证格式与随机协议；不重新生成输入，也不验证用例 schema。
    pub fn load(path: impl AsRef<Path>) -> Result<Self, RecordError> {
        let record: Record<Self> = serde_json::from_reader(BufReader::new(File::open(path)?))?;
        if record.format != FAILURE_FORMAT {
            return Err(RecordError::Format(record.format));
        }
        if record.rng != RNG_PROTOCOL {
            return Err(RecordError::Rng(record.rng));
        }
        Ok(record.failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure() -> Failure {
        Failure {
            case: "geometry.connection".into(),
            schema: "connection-v1".into(),
            seed: [7; 32],
            stream: 11,
            index: u64::MAX,
            input: vec![0x80000000, 0x7fc01234],
            actual: vec![0x7f800000, u32::MAX],
            device: "test device".into(),
            driver: "test driver".into(),
            shader_blake3: "test hash".into(),
        }
    }

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "gpu-test-record-{}-{name}.json",
            std::process::id()
        ))
    }

    #[test]
    fn json_round_trip_preserves_bits() {
        let path = path("round-trip");
        let expected = failure();
        expected.save(&path).unwrap();
        let actual = Failure::load(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn unknown_format_is_rejected() {
        let path = path("format");
        let record = Record {
            format: "unknown".into(),
            rng: RNG_PROTOCOL.into(),
            failure: failure(),
        };
        serde_json::to_writer(File::create(&path).unwrap(), &record).unwrap();
        let result = Failure::load(&path);
        std::fs::remove_file(path).unwrap();
        assert!(matches!(result, Err(RecordError::Format(_))));
    }

    #[test]
    fn unknown_rng_protocol_is_rejected() {
        let path = path("rng");
        let record = Record {
            format: FAILURE_FORMAT.into(),
            rng: "unknown".into(),
            failure: failure(),
        };
        serde_json::to_writer(File::create(&path).unwrap(), &record).unwrap();
        let result = Failure::load(&path);
        std::fs::remove_file(path).unwrap();
        assert!(matches!(result, Err(RecordError::Rng(_))));
    }
}
