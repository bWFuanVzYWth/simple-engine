//! 单层二维 KTX2 数据纹理的无损读写，坐标原点在左上角。
//! RGB32F 的字节是逐行交错的小端浮点数。线性通道可以表示卷积权重，读写时不作色彩变换。
//! 原生写入使用 Zstandard 22。每个 mip 层的载荷遵守 [KTX 2.0](https://registry.khronos.org/KTX/specs/2.0/ktxspec.v2.html)。

use ktx2::{Format, Reader, SupercompressionScheme, TransferFunction};
use std::io;

/// 文件、图像布局或压缩数据不符合调用约定。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// KTX2 的头、级别索引或数据格式描述符不可读。
    #[error("解析 KTX2 失败")]
    Header(#[from] ktx2::ParseError),
    /// 图像的类型、维度、方向或展开长度不匹配。
    #[error("KTX2 图像的格式、方向、尺寸或数据长度不匹配")]
    Layout,
    /// 输出空间不足。
    #[error("分配 KTX2 图像失败")]
    Allocation(#[from] std::collections::TryReserveError),
    /// Zstd 解码、编码或输出失败。
    #[error("读写 KTX2 纹理失败")]
    Io(#[from] io::Error),
}

fn rgb_size([width, height]: [u32; 2]) -> Result<usize, Error> {
    if width == 0 || height == 0 {
        return Err(Error::Layout);
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(12))
        .ok_or(Error::Layout)
}

/// 读取单层二维线性 RGB32F 的原始小端字节。调用者给出期望的尺寸作为输出分配上限。
/// 支持未压缩或 Zstd 超压缩的 KTX2。资产坐标采用向右、向下的行序。
/// # Errors
/// 文件损坏、格式或尺寸不符、压缩数据错误、内存不足时返回错误。
pub fn read_rgb32f(file: &[u8], extent: [u32; 2]) -> Result<Vec<u8>, Error> {
    let size = rgb_size(extent)?;
    let reader = Reader::new(file)?;
    let h = reader.header();
    let incompatible_axes = reader.key_value_data().any(|(key, value)| {
        let v = value.strip_suffix(&[0]).unwrap_or(value);
        match key {
            "KTXorientation" => v != b"rd",
            "KTXswizzle" => v.len() != 4 || &v[..3] != b"rgb",
            _ => false,
        }
    });
    if h.format != Some(Format::R32G32B32_SFLOAT)
        || h.type_size != 4
        || [h.pixel_width, h.pixel_height] != extent
        || h.pixel_depth != 0
        || h.layer_count != 0
        || h.face_count != 1
        || h.level_count != 1
        || reader.transfer_function() != Some(TransferFunction::Linear)
        || incompatible_axes
    {
        return Err(Error::Layout);
    }
    let level = reader.levels().next().expect("已解析一个 mip 层");
    if level.uncompressed_byte_length != size as u64 {
        return Err(Error::Layout);
    }
    let mut data = Vec::new();
    data.try_reserve_exact(size)?;
    match h.supercompression_scheme {
        None if level.data.len() == size => data.extend_from_slice(level.data),
        Some(SupercompressionScheme::Zstandard) => {
            data.resize(size, 0);
            inflate(level.data, &mut data)?;
        }
        _ => return Err(Error::Layout),
    }
    Ok(data)
}

#[cfg(not(target_arch = "wasm32"))]
fn inflate(input: &[u8], output: &mut [u8]) -> Result<(), Error> {
    if zstd::bulk::decompress_to_buffer(input, output)? != output.len() {
        return Err(Error::Layout);
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn inflate(mut input: &[u8], mut output: &mut [u8]) -> Result<(), Error> {
    use ruzstd::decoding::{
        StreamingDecoder,
        errors::{FrameDecoderError, ReadFrameHeaderError},
    };
    while !input.is_empty() {
        let mut reader = match StreamingDecoder::new(&mut input) {
            Ok(reader) => reader,
            Err(FrameDecoderError::ReadFrameHeaderError(ReadFrameHeaderError::SkipFrame {
                length,
                ..
            })) => {
                input = input.get(length as usize..).ok_or(Error::Layout)?;
                continue;
            }
            Err(error) => return Err(io::Error::other(error).into()),
        };
        io::copy(&mut reader, &mut output)?;
        let frame = reader.into_frame_decoder();
        if let Some(stored) = frame.get_checksum_from_data()
            && frame.get_calculated_checksum() != Some(stored)
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Zstd 帧校验失败").into());
        }
    }
    if !output.is_empty() {
        return Err(Error::Layout);
    }
    Ok(())
}

/// 把一张原始 RGB32F 像面写为标准 KTX2。每个浮点位，包括有符号零，均不变。
/// 写入线性传递函数、未指定的色度基准和明确的左上原点。高压缩等级只影响离线写入。
/// # Errors
/// 零尺寸、溢出、像素长度错误、压缩或写出失败时返回错误。调用者负责发布完整输出。
#[cfg(not(target_arch = "wasm32"))]
pub fn write_rgb32f(
    mut output: impl io::Write,
    extent: [u32; 2],
    bytes: &[u8],
) -> Result<(), Error> {
    use ktx2::{
        Header, Index, LevelIndex,
        dfd::{Basic, Block},
    };
    if rgb_size(extent)? != bytes.len() {
        return Err(Error::Layout);
    }
    let format = Format::R32G32B32_SFLOAT;
    let (mut basic, type_size) = Basic::from_format(format).expect("RGB32F 有标准 Khronos 描述符");
    basic.color_primaries = None;
    let block = Block::Basic(basic);
    let mut dfd = vec![0; 4 + block.serialized_length()];
    let dfd_length = dfd.len() as u32;
    dfd[..4].copy_from_slice(&dfd_length.to_le_bytes());
    block.to_bytes(&mut dfd[4..]);
    let mut metadata = Vec::new();
    for entry in [&b"KTXorientation\0rd\0"[..], b"KTXwriter\0simple_engine\0"] {
        metadata.extend_from_slice(&(entry.len() as u32).to_le_bytes());
        metadata.extend_from_slice(entry);
        metadata.resize(metadata.len().next_multiple_of(4), 0);
    }
    let mut encoder = zstd::bulk::Compressor::new(22)?;
    encoder.set_parameter(zstd::zstd_safe::CParameter::ChecksumFlag(true))?;
    let encoded = encoder.compress(bytes)?;
    let dfd_offset = (Header::LENGTH + LevelIndex::LENGTH) as u32;
    let kvd_offset = dfd_offset + dfd_length;
    let kvd_length = metadata.len() as u32;
    let header = Header {
        format: Some(format),
        type_size,
        pixel_width: extent[0],
        pixel_height: extent[1],
        pixel_depth: 0,
        layer_count: 0,
        face_count: 1,
        level_count: 1,
        supercompression_scheme: Some(SupercompressionScheme::Zstandard),
        index: Index {
            dfd_byte_offset: dfd_offset,
            dfd_byte_length: dfd_length,
            kvd_byte_offset: kvd_offset,
            kvd_byte_length: kvd_length,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };
    let level = LevelIndex {
        byte_offset: u64::from(kvd_offset + kvd_length),
        byte_length: encoded.len() as u64,
        uncompressed_byte_length: bytes.len() as u64,
    };
    output.write_all(&header.as_bytes())?;
    output.write_all(&level.as_bytes())?;
    output.write_all(&dfd)?;
    output.write_all(&metadata)?;
    output.write_all(&encoded)?;
    Ok(())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn preserves_all_float_bits() -> Result<(), Error> {
        let bits = [
            0x80000000u32,
            0x7fc00123,
            0x00000001,
            0x7f800000,
            0xff800000,
            0x3f800000,
        ]
        .map(u32::to_le_bytes);
        let mut file = Vec::new();
        write_rgb32f(&mut file, [2, 1], bits.as_flattened())?;
        assert_eq!(read_rgb32f(&file, [2, 1])?, bits.as_flattened());

        Ok(())
    }
    #[test]
    fn rejects_damaged_zstd_checksum() -> Result<(), Error> {
        let mut file = Vec::new();
        write_rgb32f(&mut file, [1, 1], &[0; 12])?;
        *file.last_mut().expect("纹理含完整的帧校验码") ^= 0x40;
        assert!(read_rgb32f(&file, [1, 1]).is_err());
        Ok(())
    }
}
