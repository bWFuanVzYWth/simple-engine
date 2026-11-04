//! 资源重载使用的显式 Slang 编译器与内容缓存；不读取 Cargo `OUT_DIR`，不输出 Cargo 指令。
//!
//! 一次所有者保存已确认的编译器身份。每次编译重新读取受控源码根目录，
//! 目录内模块、include、所有 recipe 参数及输出名共同参与缓存键。
//! 调用方须将所有可导入源码包含在 `source_roots` 或 `includes`；不控制外部源文件的修改。
//! 缓存同时保存编译器身份、参数、源码指纹和输出校验；命中时校验实际字节。
//! 失败保留诊断，不替换已有产物，应用负责在整代资源成功后原子发布。
//! 使用与构建入口相同的 target/profile 调试策略；只保证 SPIR-V 有效性，不执行应用的物理特化审计。

use super::{Compiler, Error, Recipe, Target, debug_argument, field, files, read, validate_recipe};
use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// 一次受控编译的实际输出；所有者包含字节，不依赖后续缓存文件的寿命。
pub struct Artifact {
    bytes: Vec<u8>,
    fingerprint: String,
    path: PathBuf,
    cache_hit: bool,
}

impl Artifact {
    /// Slang 直接生成的最终字节；native SPIR-V 不经过其他编译器或插桩。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 编译器、源码、recipe 和参数的 BLAKE3 指纹，不是跨工具链 ABI。
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// 通过内容校验的缓存产物位置；同名 `.txt` 文件保存编译身份和完整诊断。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 本次是否直接复用了身份及内容均匹配的缓存。
    pub const fn cache_hit(&self) -> bool {
        self.cache_hit
    }
}

/// 显式所有的运行时编译器；不保存应用材质、设备资源或隐含全局缓存。
pub struct RuntimeCompiler {
    compiler: Compiler,
    cache: PathBuf,
    includes: Vec<PathBuf>,
    sources: Vec<PathBuf>,
}

impl RuntimeCompiler {
    /// 按本 crate 的规则发现编译器，指定受控源码和显式缓存目录。
    ///
    /// `includes` 也参与源码身份；所有路径以调用时的工作目录解析为绝对路径。
    /// 后续改装编译器发行包时须重建本所有者，不在一次资源重载中混用工具链。
    /// # Errors
    /// 保留编译器发现、版本查询与文件系统操作的 [`Error`] 来源。
    pub fn new(
        cache: impl AsRef<Path>,
        includes: &[&Path],
        source_roots: &[&Path],
    ) -> Result<Self, Error> {
        Self::with_compiler(
            Compiler::find(false)?,
            cache.as_ref(),
            includes,
            source_roots,
        )
    }

    /// 使用显式安装的 Slang 可执行文件，路径必须为绝对路径。
    /// 适用于应用自带工具链；不会通过 `SLANGC`、`VULKAN_SDK` 或 `PATH` 选择编译器。
    /// 其余源码及缓存契约同 [`Self::new`]。
    /// # Errors
    /// 路径不是绝对路径、工具链不可用或目录配置无效时，返回对应的 [`Error`]。
    pub fn from_executable(
        executable: &Path,
        cache: impl AsRef<Path>,
        includes: &[&Path],
        source_roots: &[&Path],
    ) -> Result<Self, Error> {
        if !executable.is_absolute() {
            return Err(Error::Configuration(
                "Slang 可执行文件须使用绝对路径".into(),
            ));
        }
        Self::with_compiler(
            Compiler::inspect(executable, |_| {})?,
            cache.as_ref(),
            includes,
            source_roots,
        )
    }

    fn with_compiler(
        compiler: Compiler,
        cache: &Path,
        includes: &[&Path],
        source_roots: &[&Path],
    ) -> Result<Self, Error> {
        fs::create_dir_all(cache).map_err(|source| Error::Io {
            path: cache.to_owned(),
            source,
        })?;
        let cache = canonical(cache)?;
        let includes = includes
            .iter()
            .map(|path| canonical(path))
            .collect::<Result<Vec<_>, _>>()?;
        let sources = source_roots
            .iter()
            .map(|path| canonical(path))
            .collect::<Result<Vec<_>, _>>()?;
        for root in includes.iter().chain(&sources) {
            if cache.starts_with(root) {
                return Err(Error::SourceCacheOverlap {
                    cache: cache.clone(),
                    source_root: root.clone(),
                });
            }
        }
        Ok(Self {
            compiler,
            cache,
            includes,
            sources,
        })
    }

    /// 编译一个入口或复用已校验缓存；宏定义通过 `recipe.args` 明确传入。
    ///
    /// 输出位于内容寻址缓存，不使用 `recipe.output` 指定的宿主目录。
    /// 该字段仍完整参与身份，缓存文件仅保留其扩展名。
    /// 同一所有者可在资源重载时连续编译多个入口，调度由调用方决定。
    /// # Errors
    /// 源码读取、缓存 IO、启动编译器或 Slang 诊断失败时返回带底层来源的 [`Error`]。
    pub fn compile(&self, recipe: &Recipe<'_>) -> Result<Artifact, Error> {
        validate_recipe(recipe)?;
        let source = canonical(Path::new(recipe.source))?;
        let mut roots = self.sources.clone();
        roots.extend(self.includes.iter().cloned());
        roots.push(source.clone());
        let mut source_hash = blake3::Hasher::new();
        field(&mut source_hash, self.compiler.identity.as_bytes());
        for path in files(roots)? {
            field(&mut source_hash, path.to_string_lossy().as_bytes());
            field(&mut source_hash, &read(&path)?);
        }
        let source_hash = source_hash.finalize().to_hex().to_string();
        let mut arguments = vec![
            command_path(&source),
            "-std".into(),
            "2026".into(),
            "-O3".into(),
            debug_argument(recipe.target).into(),
        ];
        for include in &self.includes {
            arguments.extend(["-I".into(), command_path(include)]);
        }
        match recipe.target {
            Target::Spirv(profile) => arguments.extend([
                "-target".into(),
                "spirv".into(),
                "-profile".into(),
                profile.into(),
                "-emit-spirv-directly".into(),
                "-fvk-use-entrypoint-name".into(),
            ]),
            Target::Wgsl => arguments.extend(["-target".into(), "wgsl".into()]),
        }
        arguments.extend([
            "-entry".into(),
            recipe.entry.into(),
            "-stage".into(),
            recipe.stage.into(),
        ]);
        arguments.extend(recipe.args.iter().map(|argument| (*argument).to_owned()));
        if matches!(recipe.target, Target::Wgsl) {
            arguments.push("-g0".into());
        }
        let mut identity = blake3::Hasher::new();
        field(&mut identity, source_hash.as_bytes());
        field(&mut identity, recipe.output.as_bytes());
        for argument in &arguments {
            field(&mut identity, argument.as_bytes());
        }
        let fingerprint = identity.finalize().to_hex().to_string();
        let extension = match recipe.target {
            Target::Spirv(_) => "spv",
            Target::Wgsl => "wgsl",
        };
        let path = self.cache.join(format!("{fingerprint}.{extension}"));
        let checksum = self.cache.join(format!("{fingerprint}.blake3"));
        if let Some(bytes) = cached(&path, &checksum)? {
            return Ok(Artifact {
                bytes,
                fingerprint,
                path,
                cache_hit: true,
            });
        }
        let temporary = self.cache.join(format!(
            "{fingerprint}.{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        let mut command = self.compiler.command();
        command
            .args(&arguments)
            .arg("-o")
            .arg(command_path(&temporary));
        let result = command.output().map_err(|source| Error::Io {
            path: self.compiler.path.clone(),
            source,
        })?;
        let diagnostic = format!(
            "{}\n{}\n{}",
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let manifest = format!(
            "compiler={}\nversion={}\ncompiler_blake3={}\nSLANG_RUN_SPIRV_VALIDATION=1\nsource_blake3={source_hash}\nbuild_blake3={fingerprint}\narguments={arguments:?}\n{diagnostic}",
            self.compiler.path.display(),
            self.compiler.version,
            self.compiler.identity
        );
        let log = self.cache.join(format!("{fingerprint}.txt"));
        fs::write(&log, manifest).map_err(|source| Error::Io { path: log, source })?;
        if !result.status.success() {
            let _ = fs::remove_file(&temporary);
            return Err(Error::Compile {
                compiler: self.compiler.path.clone(),
                entry: recipe.entry.to_owned(),
                diagnostic,
            });
        }
        let bytes = read(&temporary)?;
        let digest = blake3::hash(&bytes).to_hex().to_string();
        // 每个临时文件独占；同身份的并发产物等价，替换只影响可再生缓存。
        replace(&temporary, &path)?;
        fs::write(&checksum, digest).map_err(|source| Error::Io {
            path: checksum,
            source,
        })?;
        Ok(Artifact {
            bytes,
            fingerprint,
            path,
            cache_hit: false,
        })
    }
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn command_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    // Slang 的 include 拼接不接受 Windows verbatim 前缀；文件身份仍使用 canonical 路径。
    if cfg!(windows) {
        if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
            return format!("\\\\{unc}");
        }
        if let Some(absolute) = text.strip_prefix("\\\\?\\") {
            return absolute.to_owned();
        }
    }
    text.into_owned()
}

fn cached(path: &Path, checksum: &Path) -> Result<Option<Vec<u8>>, Error> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    let expected = match fs::read_to_string(checksum) {
        Ok(expected) => expected,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(Error::Io {
                path: checksum.to_owned(),
                source,
            });
        }
    };
    Ok((blake3::hash(&bytes).to_hex().as_str() == expected.trim()).then_some(bytes))
}

fn replace(temporary: &Path, destination: &Path) -> Result<(), Error> {
    match fs::rename(temporary, destination) {
        Ok(()) => Ok(()),
        Err(_) if destination.is_file() && cfg!(windows) => {
            fs::remove_file(destination).map_err(|source| Error::Io {
                path: destination.to_owned(),
                source,
            })?;
            fs::rename(temporary, destination).map_err(|source| Error::Io {
                path: destination.to_owned(),
                source,
            })
        }
        Err(source) => Err(Error::Io {
            path: destination.to_owned(),
            source,
        }),
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
