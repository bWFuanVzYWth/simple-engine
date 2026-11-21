//! 可寻址的无损二进制资源包。每个条目独立使用 ZIP 的 Zstandard 22 压缩，按名称读取只展开该条目。
//! 名称是包内的逻辑路径。读取过程不会将路径解析到文件系统上。
//! 写入顺序决定目录顺序，相同输入使用固定时间戳。
//! `texture` 功能提供 KTX2 数据纹理读写，独立的纹理文件可供标准工具直接使用。

#[cfg(feature = "texture")]
pub mod texture;

use std::{
    borrow::Cow,
    collections::TryReserveError,
    io::{self, Read, Seek, Write},
};
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

/// 资源包的目录、数据或底层存储错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// ZIP 格式、重复条目、查找或压缩错误。
    #[error("资源包目录错误")]
    Archive(#[from] zip::result::ZipError),
    /// 读写数据或 CRC 校验失败。
    #[error("资源包数据读写失败")]
    Io(#[from] io::Error),
    /// 先前的条目写入已经失败，不能发布这个输出。
    #[error("资源包写入未完成")]
    Incomplete,
    /// 逻辑路径必须是非空的相对路径，使用正斜线分隔。
    #[error("无效的资源条目名")]
    Name,
    /// 条目宣告的解压长度超过调用者给出的字节上限。
    #[error("资源条目需要 {size} 字节，读取上限为 {limit}")]
    Limit {
        /// ZIP 目录记录的展开长度。
        size: u64,
        /// 调用者可接受的最大长度。
        limit: usize,
    },
    /// 条目实际长度与目录不符。
    #[error("资源条目长度与目录不符")]
    Length,
    /// 无法保留所需的输出空间。
    #[error("分配资源数据失败")]
    Allocation(#[from] TryReserveError),
}

/// 按加入顺序写入的 ZIP 包。写入失败时应放弃该包，只有 `finish` 成功后才能发布。
pub struct Writer<W: Write + Seek>(Option<ZipWriter<W>>);

impl<W: Write + Seek> Writer<W> {
    /// 从当前位置开始写入，接管新的空输出。
    pub fn new(output: W) -> Self {
        Self(Some(ZipWriter::new(output)))
    }

    /// 压缩一个具名条目，流式读到输入结束。
    /// # Errors
    /// 返回非法名称、重名、输入或目标写入错误。
    pub fn add(&mut self, name: &str, mut input: impl Read) -> Result<(), Error> {
        if name.contains(['\\', ':', '\0'])
            || name.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(Error::Name);
        }
        let options = SimpleFileOptions::default()
            .last_modified_time(zip::DateTime::default())
            .compression_method(CompressionMethod::Zstd)
            .compression_level(Some(22))
            .large_file(true);
        let mut archive = self.0.take().ok_or(Error::Incomplete)?;
        archive.start_file(name, options)?;
        io::copy(&mut input, &mut archive)?;
        self.0 = Some(archive);
        Ok(())
    }

    /// 完成中央目录并返还目标。
    /// # Errors
    /// 无法完成压缩或目录写入时返回底层错误。
    pub fn finish(self) -> Result<W, Error> {
        Ok(self.0.ok_or(Error::Incomplete)?.finish()?)
    }
}

/// 已读取目录的资源包，可多次独立读取不同条目。
pub struct Pack<R: Read + Seek>(ZipArchive<R>);

impl<R: Read + Seek> Pack<R> {
    /// 解析 ZIP 目录并接管可寻址的数据源，条目数据仍留在源中。
    /// # Errors
    /// 目录损坏、格式不可读或数据源读取失败时返回错误。
    pub fn open(input: R) -> Result<Self, Error> {
        Ok(Self(ZipArchive::new(input)?))
    }

    /// 按目录顺序列出条目。不能解码的名称在对应位置返回格式错误。
    pub fn names(&self) -> impl Iterator<Item = Result<Cow<'_, str>, Error>> {
        self.0.file_names().map(|name| name.map_err(Into::into))
    }

    /// 将指定条目流式写到调用者选择的目标，读取到末尾校验 CRC 和展开长度。
    /// # Errors
    /// 读取、解压、写出或目录长度校验失败时，目标可能已包含部分数据。
    pub fn copy_to(&mut self, name: &str, mut output: impl Write) -> Result<u64, Error> {
        let mut entry = self.0.by_name(name)?;
        let size = entry.size();
        let count = io::copy(&mut entry.by_ref().take(size), &mut output)?;
        if count != size || entry.read(&mut [0])? != 0 {
            return Err(Error::Length);
        }
        Ok(count)
    }

    /// 将一个定长条目直接解压进调用者的缓冲，读取到末尾校验 CRC。
    /// # Errors
    /// 条目大小必须恰好等于缓冲长度。解压或校验失败时缓冲可能已部分写入。
    pub fn read_into(&mut self, name: &str, output: &mut [u8]) -> Result<(), Error> {
        let entry = self.0.by_name(name)?;
        if entry.size() != output.len() as u64 {
            return Err(Error::Length);
        }
        fill(entry, output)
    }

    /// 读取指定条目并校验 CRC，最多分配 `limit` 字节的输出。
    /// # Errors
    /// 条目缺失、超过上限、长度不符、校验失败或分配失败时返回错误。
    pub fn read(&mut self, name: &str, limit: usize) -> Result<Vec<u8>, Error> {
        let entry = self.0.by_name(name)?;
        let size = entry.size();
        if size > limit as u64 {
            return Err(Error::Limit { size, limit });
        }
        let mut data = Vec::new();
        data.try_reserve_exact(size as usize)?;
        data.resize(size as usize, 0);
        fill(entry, &mut data)?;
        Ok(data)
    }
}

fn fill(mut entry: impl Read, output: &mut [u8]) -> Result<(), Error> {
    entry.read_exact(output)?;
    if entry.read(&mut [0])? != 0 {
        return Err(Error::Length);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn entries_are_independently_bounded() -> Result<(), Error> {
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        writer.add("world/sky", &[0xff; 8192][..])?;
        writer.add("empty", &[][..])?;
        let mut pack = Pack::open(writer.finish()?)?;
        assert!(matches!(
            pack.read("world/sky", 8191),
            Err(Error::Limit { .. })
        ));
        assert!(pack.read("empty", 0)?.is_empty());
        assert_eq!(pack.read("world/sky", 8192)?, [0xff; 8192]);
        Ok(())
    }

    #[test]
    fn unfinished_input_cannot_publish_a_partial_entry() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("模拟读取中断"))
            }
        }
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.add("asset", (&b"first"[..]).chain(Broken)),
            Err(Error::Io(_))
        ));
        assert!(matches!(writer.finish(), Err(Error::Incomplete)));
    }

    #[test]
    fn crc_damage_is_reported() -> Result<(), Error> {
        let mut writer = Writer::new(Cursor::new(Vec::new()));
        writer.add("x", &b"content"[..])?;
        let mut data = writer.finish()?.into_inner();
        let index = data
            .windows(4)
            .rposition(|w| w == b"PK\x01\x02")
            .expect("测试包有中央目录");
        data[index + 16] ^= 128;
        let mut pack = Pack::open(Cursor::new(data))?;
        assert!(pack.read("x", 7).is_err());
        Ok(())
    }
}
