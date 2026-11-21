//! 从命令行打包、列出和提取资源。先完成同目录临时文件，再原子替换输出。
use resource_pack::{Pack, Writer};
use std::{
    env,
    error::Error,
    fs::File,
    io::{BufReader, Write},
    path::Path,
};
use tempfile::NamedTempFile;

fn output(path: &Path) -> std::io::Result<NamedTempFile> {
    NamedTempFile::new_in(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, dst, entries @ ..] if cmd == "pack" && !entries.is_empty() => {
            let mut tmp = output(Path::new(dst))?;
            let mut writer = Writer::new(tmp.as_file_mut());
            for entry in entries {
                let (name, file) = entry.split_once('=').ok_or("条目应写作 包内名称=源文件")?;
                writer.add(name, BufReader::new(File::open(file)?))?;
            }
            writer.finish()?;
            tmp.persist(dst)?;
        }
        [cmd, src] if cmd == "list" => {
            let mut listing = std::io::stdout().lock();
            for name in Pack::open(BufReader::new(File::open(src)?))?.names() {
                writeln!(listing, "{}", name?)?;
            }
        }
        [cmd, src, name, dst] if cmd == "get" => {
            let mut archive = Pack::open(BufReader::new(File::open(src)?))?;
            let mut tmp = output(Path::new(dst))?;
            archive.copy_to(name, tmp.as_file_mut())?;
            tmp.flush()?;
            tmp.persist(dst)?;
        }
        #[cfg(not(target_arch = "wasm32"))]
        [cmd, width, height, src, dst] if cmd == "encode-rgb32f" || cmd == "decode-rgb32f" => {
            let extent = [width.parse()?, height.parse()?];
            let input = std::fs::read(src)?;
            let mut tmp = output(Path::new(dst))?;
            if cmd == "encode-rgb32f" {
                resource_pack::texture::write_rgb32f(tmp.as_file_mut(), extent, &input)?;
            } else {
                tmp.write_all(&resource_pack::texture::read_rgb32f(&input, extent)?)?;
            }
            tmp.persist(dst)?;
        }
        _ => {
            return Err(concat!(
                "用法：resource_pack pack 输出.zip 名称=文件… | list 包.zip | get 包.zip 名称 输出文件",
                " | encode-rgb32f 宽 高 原始输入 KTX2输出 | decode-rgb32f 宽 高 KTX2输入 原始输出"
            )
            .into());
        }
    }
    Ok(())
}
