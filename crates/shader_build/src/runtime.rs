//! 资源准备使用的显式Slang编译器；与Cargo共用真实依赖内容缓存。
//!
//! 调用方先向[`RuntimeBatch`]登记所有独立入口，再指定并行度一次编译。
//! 每批扫描一次受控输入，缓存只依赖Slang报告的import/include闭包及搜索目录清单；
//! 不导入的已有文件内容变化不会使该入口失效，新增同名模块仍会失效。
//! 缓存校验工具链、参数、依赖及main/完整审计字节，失败不返回部分批次。
//! 普通release的同次sidecar在缓存恢复/编译时严格配对，之后只保留路径与摘要；
//! [`Artifact::read_audit`]明确按需IO，主码自身不依赖缓存文件寿命。
//! 调用方保持源码在批次期间不变，整代应用资源成功后再发布；缓存不代替应用ABI审查。
//! 不读取Cargo OUT_DIR、jobserver或并行度环境变量，也不输出Cargo指令。

use super::{
    Compiler, Error, Recipe, batch, canonical_path, collect_input_files, command_path,
    compiler_arguments, read_file, validate_recipe,
};
use std::{
    borrow::Cow,
    fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);

/// 一次受控编译的实际输出；所有者包含字节，不依赖后续缓存文件的寿命。
pub struct Artifact {
    bytes: Vec<u8>,
    audit: Option<batch::AuditFile>,
    fingerprint: String,
    path: PathBuf,
    cache_hit: bool,
}

impl Artifact {
    /// Slang 直接生成的最终字节；native SPIR-V 不经过其他编译器或插桩。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// 按需读取完整审计字节并核验内容身份；inline产物借用自身，不做IO。
    ///
    /// paired副本已在编译/缓存恢复时与main严格配对；本所有者只保留其路径及摘要，
    /// 不为未使用审计的运行时常驻完整debug副本。主码始终由本对象拥有、不依赖缓存寿命。
    /// # Errors
    /// paired副本的缓存/uncached文件被清理、改写或无法读取时返回错误。
    /// 成功读取的副本由返回的Cow独占所有；调用者审查后可立即释放。
    pub fn read_audit(&self) -> Result<Cow<'_, [u8]>, Error> {
        let Some(audit) = &self.audit else {
            return Ok(Cow::Borrowed(&self.bytes));
        };
        let bytes = read_file(&audit.path)?;
        if blake3::hash(&bytes).to_hex().as_str() != audit.digest {
            return Err(Error::AuditPair("审计副本内容身份已改变".into()));
        }
        Ok(Cow::Owned(bytes))
    }

    /// 编译器、参数、实际依赖及产物的BLAKE3身份，不是跨工具链ABI。
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// 已校验产物的位置；缓存对象同目录的diagnostic.txt保存首次实际编译诊断。
    /// 未声明依赖的产物不入缓存，其诊断在本路径替换扩展名为txt的文件中。
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
            Compiler::inspect(executable, None, |_| {})?,
            cache.as_ref(),
            includes,
            source_roots,
        )
    }

    /// 使用可搬移资源包中的编译器和源码；包内文件用相对名和实际内容标识。
    /// `executable` 相对于 `package`，外部 include 仍由其绝对路径标识。调用方应使用
    /// 与提取目录无关的发行源码和编译参数。保留源码调试信息时缓存键同时绑定提取目录。
    /// # Errors
    /// 包目录、指定工具、缓存或源码配置无效时返回对应的 [`Error`]。
    pub fn from_package(
        package: &Path,
        executable: &Path,
        cache: impl AsRef<Path>,
        includes: &[&Path],
        source_roots: &[&Path],
    ) -> Result<Self, Error> {
        let package = canonical_path(package)?;
        let tool = canonical_path(&package.join(executable))?;
        if !tool.starts_with(&package) {
            return Err(Error::Configuration("发行编译器须位于资源包中".into()));
        }
        Self::with_compiler(
            Compiler::inspect(&tool, Some(package), |_| {})?,
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
        let cache = canonical_path(cache)?;
        let includes = includes
            .iter()
            .map(|path| canonical_path(path))
            .collect::<Result<Vec<_>, _>>()?;
        let sources = source_roots
            .iter()
            .map(|path| canonical_path(path))
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

    /// 建立显式批次；此时只登记参数，不启动编译器或读取整套源码。
    pub fn batch(&self) -> RuntimeBatch<'_> {
        RuntimeBatch {
            compiler: self,
            identity: SERIAL.fetch_add(1, Ordering::Relaxed),
            tasks: Vec::new(),
        }
    }

    /// 串行编译单个入口；多个已知入口应通过[`Self::batch`]共享源码扫描和调度。
    /// `recipe.output`只是逻辑名称，不影响实际编译身份或输出目录。
    /// # Errors
    /// 与[`RuntimeBatch::queue`]及[`RuntimeBatch::compile`]相同。
    pub fn compile(&self, recipe: &Recipe<'_>) -> Result<Artifact, Error> {
        let mut batch = self.batch();
        let ticket = batch.queue(recipe)?;
        batch.compile(NonZeroUsize::MIN)?.take(ticket)
    }
}

/// 登记入口的唯一取件凭证；只能从产生它的批次领取一次产物。
pub struct RuntimeTicket {
    identity: u64,
    index: usize,
}

/// 尚未执行的资源编译批次；输入参数由本对象拥有，编译器短期借用。
pub struct RuntimeBatch<'a> {
    compiler: &'a RuntimeCompiler,
    identity: u64,
    tasks: Vec<batch::Task>,
}

impl RuntimeBatch<'_> {
    /// 已登记的入口数量，包含显式重复请求。
    pub const fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// 登记入口并复制参数，返回取件凭证；不会编译或发布应用资源。
    /// # Errors
    /// 路径、验证选项或输出参数不受支持时返回[`Error`]，既有登记保持有效。
    pub fn queue(&mut self, recipe: &Recipe<'_>) -> Result<RuntimeTicket, Error> {
        validate_recipe(recipe)?;
        let source = canonical_path(Path::new(recipe.source))?;
        let arguments = compiler_arguments(
            recipe,
            command_path(&source),
            self.compiler.includes.iter().map(|path| command_path(path)),
        );
        let index = self.tasks.len();
        let fallback = self.compiler.cache.join(format!(
            "uncached-{}-{}-{index}",
            std::process::id(),
            self.identity
        ));
        self.tasks
            .push(batch::Task::cached(recipe.entry, arguments, fallback)?);
        Ok(RuntimeTicket {
            identity: self.identity,
            index,
        })
    }

    /// 统一扫描输入后以显式非零worker上限编译；按登记顺序保存结果。
    /// 一项失败后停止分配新任务并等待已启动任务结束，不返回部分批次。
    /// 同身份请求由缓存锁去重；不再向调用者或内核隐藏嵌套并行。
    /// # Errors
    /// 源码/缓存IO、编译或线程失败保留[`Error`]来源；成功缓存可供下次重试复用。
    pub fn compile(self, workers: NonZeroUsize) -> Result<CompiledBatch, Error> {
        let tracked = collect_input_files(self.compiler.sources.clone())?;
        let output = batch::compile_runtime(
            &self.compiler.compiler,
            &self.compiler.cache,
            &self.tasks,
            &tracked,
            workers,
        )?;
        let artifacts = output
            .compiled
            .into_iter()
            .map(|compiled| {
                Some(Artifact {
                    bytes: compiled.bytes,
                    audit: compiled.audit,
                    fingerprint: compiled.fingerprint,
                    path: compiled.path,
                    cache_hit: compiled.cache_hit,
                })
            })
            .collect();
        Ok(CompiledBatch {
            identity: self.identity,
            artifacts,
            statistics: output.statistics,
        })
    }
}

/// 一次完整编译的自有产物；取走的字节不依赖缓存文件或编译器寿命。
pub struct CompiledBatch {
    identity: u64,
    artifacts: Vec<Option<Artifact>>,
    statistics: batch::CompileStatistics,
}

impl CompiledBatch {
    /// 批次实际命中和准备成本，不包含调用方的后续GPU管线创建。
    pub const fn statistics(&self) -> batch::CompileStatistics {
        self.statistics
    }

    /// 用同批次凭证移动一个产物；不会复制shader字节。
    /// # Errors
    /// 凭证来自另一批次或已消费时返回配置错误，不移走其他产物。
    pub fn take(&mut self, ticket: RuntimeTicket) -> Result<Artifact, Error> {
        if ticket.identity != self.identity {
            return Err(Error::Configuration("shader凭证不属于此编译批次".into()));
        }
        self.artifacts
            .get_mut(ticket.index)
            .and_then(Option::take)
            .ok_or_else(|| Error::Configuration("shader产物已消费或不存在".into()))
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
