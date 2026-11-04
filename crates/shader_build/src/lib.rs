//! Slang 构建入口；入口、浮点模式和 capability 由各应用的 recipe 明确指定。
//!
//! 默认使用 `-std 2026 -O3`；SPIR-V 在 `release-lto` 使用 `-g0`，其他 profile 使用 `-g3`。
//! WGSL 始终使用 `-g0`，避免 Slang 调试保留的无效接口辅助类型。`Recipe::args` 可后置覆盖优化级别。
//! 模块主文件显式声明 `module`，跨模块接口使用 `public`。
//! native 直接生成 SPIR-V，固定启用 Slang 内部 SPIRV-Tools 验证，宿主环境不能关闭。
//! Web 使用 WGSL target；构建工具不依赖渲染后端或内置算法。
//!
//! # 编译器与增量构建
//!
//! 按 `SLANGC` → `VULKAN_SDK/Bin/slangc.exe`（其他平台为 `bin/slangc`）→ `PATH` 查找。
//! `SLANGC` 可为路径或命令名，执行 canonical 路径。身份包含路径、`-version`、可执行文件、
//! bin/lib 内的 `slang*` / `libslang*` 组件与标准模块，以及动态库搜索目录中的 Slang 组件。
//! 应使用完整发行包；Windows 依赖库置于 `slangc.exe` 同目录，见
//! [Slang 安装说明](https://github.com/shader-slang/slang/blob/master/docs/user-guide/01-get-started.md)。
//!
//! Cargo 跟踪源码、include、recipe、构建工具、安装目录和相关环境；安装目录的组件增删
//! 也会触发重建。PATH 查找跟踪经过的现有目录，以发现更早位置新增的编译器；这可能因
//! 无关文件变更而重建，指定 `SLANGC` 可缩小范围。依赖检测遵循
//! [Cargo mtime 规则](https://doc.rust-lang.org/cargo/reference/build-scripts.html#rerun-if-changed)，
//! 不保证发现保留旧时间戳的外部替换。
//! 构建脚本重跑时不再全量编译：每个入口按编译器内容身份、完整语义参数、真实依赖内容
//! 查缓存。Slang `-depfile` 提供 import/include 闭包；搜索目录的相关扩展名清单也参与核验，
//! 新增同名模块会使缓存失效。宿主源码变动仍更新构建身份，应用 ABI／物理审计每次照常执行。
//! `SHADER_BUILD_CACHE` 可指定缓存路径，默认在 Cargo 构建目录的 `shader-cache`，跨 profile、
//! target 和 `OUT_DIR` 共享。单入口和反射采用内容校验，独占请求锁、临时文件与原子索引提交；
//! 损坏或不完整的缓存重新编译，失败不写入缓存。保留历史依赖版本，修改后还原或切分支可复用旧产物；
//! 热命中只读当前索引，输入变化时才枚举历史候选。缓存不是不可信输入的安全隔离边界。
//!
//! 仅把源码、`-I` 和反射参数中的 `OUT_DIR` 规范为稳定标记，实际生成模块内容仍参与身份；
//! 其余参数和 include 顺序不变。命中时不改写产物，`-g3` 可保留首次编译的生成源绝对路径，
//! 不保证该旧目录仍存在。缓存对象的 `diagnostic.txt` 保存首次实际命令和完整编译诊断；
//! `shader-build.txt` 描述本次 recipe、命中统计与批量准备／调度／写回耗时，不声称实际重编译。
//!
//! 构建调度默认至多8个 scoped worker，可用正整数 `SHADER_BUILD_JOBS` 调整上限；
//! Cargo 内额外 worker 动态借用 jobserver 令牌，与其他 crate 共享总并行度。
//! 一个任务独占 Slang 子进程和输出，输入快照共享只读，线程不再嵌套调度。
//! 工作集包括并发编译器、各任务产物和保留的缓存；缓存可随构建目录清理。
//! 构建期间不应并发编辑输入；已知输入中途变化会报错，越出声明根的依赖只编译、不缓存。
//!
//! 资源重载使用 [`RuntimeCompiler`]：显式源码／include／缓存目录，不读取 `OUT_DIR` 或输出
//! Cargo 指令；产物含源码和工具链内容指纹，保留参数及编译诊断，并校验缓存实际字节。
//!
//! # 构建产物身份
//!
//! [`build`] 通过 `cargo::rustc-env` 提供下列字段，并在 `OUT_DIR/shader-build.txt` 保存
//! 编译器身份、固定验证策略和各入口参数。指纹采用长度分隔字段，路径与 recipe 顺序参与身份，
//! 用于区分构建，不是跨工具链稳定 ABI。
//!
//! | 字段 | 内容 |
//! | --- | --- |
//! | `SLANG_VERSION` | 编译器实际版本输出 |
//! | `SLANG_COMPILER` | canonical 可执行路径 |
//! | `SLANG_COMPILER_BLAKE3` | 编译器及 Slang 组件身份 |
//! | `SOURCE_BLAKE3` | 编译器、shader、构建工具及指定宿主源码 |
//! | `SHADER_BUILD_BLAKE3` | 编译器、shader、构建工具及实际 recipe 参数 |
//! | `SPIRV_BLAKE3` | native 各入口及实际字节；纯 WGSL 构建不提供 |
//! | `SHADER_OUTPUT_BLAKE3` | 全部发布输出名及实际产物字节 |
//! | `AUDIT_SPIRV_BLAKE3` | `build_audited` 额外审计副本的输出名和字节；没有副本时不提供 |
//!
//! # build.rs 示例
//!
//! ```no_run
//! use shader_build::{Recipe, Target, build};
//! build(&[Recipe {
//!     source: "shaders/main.slang",
//!     entry: "compute_main",
//!     stage: "compute",
//!     output: "compute.spv",
//!     target: Target::Spirv("spirv_1_5"),
//!     args: &["-fp-mode", "precise"],
//! }], &["shaders"], &["src"])?;
//! # Ok::<(), shader_build::Error>(())
//! ```

use std::{
    collections::BTreeSet,
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

mod batch;
#[cfg(test)]
mod profile;
mod runtime;
pub use runtime::{Artifact, RuntimeCompiler};

/// Slang 输出语言及对应 profile；不在构建后经过其他编译器重写。
#[derive(Clone, Copy)]
pub enum Target<'a> {
    /// 直接生成 SPIR-V，携带 Slang profile（如 `spirv_1_5`）。
    Spirv(&'a str),
    /// 生成浏览器 WebGPU 使用的 WGSL。
    Wgsl,
}

/// 当前编译是否使用 Cargo 的精确 `release-lto` profile；build 与运行时编译采用相同策略。
/// 根据 Cargo 包槽的 profile 目录识别；自定义 build-dir 和跨目标布局同样适用，未知布局构建失败。
pub const fn release_lto() -> bool {
    cfg!(shader_release_lto)
}

const fn debug_argument(target: Target<'_>) -> &'static str {
    match target {
        Target::Wgsl => "-g0",
        Target::Spirv(_) if release_lto() => "-g0",
        Target::Spirv(_) => "-g3",
    }
}

/// 单个入口的编译声明；参数顺序参与构建身份。
pub struct Recipe<'a> {
    /// 相对当前构建脚本工作目录的主文件路径。
    pub source: &'a str,
    /// 模块导出的入口名称。
    pub entry: &'a str,
    /// Slang stage 名称，如 `compute`、`vertex`、`fragment`。
    pub stage: &'a str,
    /// `OUT_DIR` 内的输出路径；父目录须已存在。
    pub output: &'a str,
    /// 输出语言及 profile。
    pub target: Target<'a>,
    /// 后置的原始 Slang 参数；可覆盖默认优化级别，浮点模式、capability 等须明确指定。
    /// SPIR-V 不接受跳过验证或未链接库参数。
    pub args: &'a [&'a str],
}

/// 编译器发现、文件 IO 或入口编译失败。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 与指定路径关联的 IO 失败。
    #[error("{path}：{source}")]
    Io {
        /// 失败操作涉及的文件或目录。
        path: PathBuf,
        /// 操作系统或进程启动错误。
        #[source]
        source: io::Error,
    },
    /// 查找不到请求的编译器路径或命令名。
    #[error("找不到 Slang 编译器 {0}；请设置 SLANGC、VULKAN_SDK 或 PATH")]
    Compiler(PathBuf),
    /// 调用环境不是提供 `OUT_DIR` 的 Cargo 构建脚本。
    #[error("Cargo 未设置 OUT_DIR")]
    OutputDirectory,
    /// 运行时缓存位于受控源码根目录内，产物会反过来污染源码身份。
    #[error("shader 缓存 {cache} 不能位于源码根目录 {source_root} 内")]
    SourceCacheOverlap {
        /// 实际缓存绝对路径。
        cache: PathBuf,
        /// 包含缓存的源码或 include 根目录。
        source_root: PathBuf,
    },
    /// SPIR-V recipe 试图跳过验证或生成不适用于 Vulkan 验证的未链接库。
    #[error("SPIR-V 构建必须启用完整产物验证")]
    Validation,
    /// 并行度、输出声明或缓存环境配置无效。
    #[error("shader 构建配置无效：{0}")]
    Configuration(String),
    /// 编译期间源码发生改变，不能把产物发布为原输入的缓存。
    #[error("shader 编译期间输入改变：{0}")]
    InputChanged(PathBuf),
    /// 构建任务线程异常退出。
    #[error("shader 构建任务异常退出")]
    Worker,
    /// 编译器执行成功启动，但返回非零状态。
    #[error("Slang {entry} 失败（{compiler}）：\n{diagnostic}")]
    Compile {
        /// 实际执行的 canonical 编译器路径。
        compiler: PathBuf,
        /// 失败入口名称，版本查询失败时为 `-version`。
        entry: String,
        /// 退出状态、stdout 与 stderr 的完整诊断。
        diagnostic: String,
    },
}

/// 编译所有入口并向 Cargo 发布依赖跟踪、构建身份及实际产物。
///
/// `source_roots` 补充基准身份所需的宿主源码；shader 与 include 目录会自动跟踪。
/// 未带目录的入口只跟踪文件，其他本地模块目录须加入 includes。
/// Cargo 决定是否重跑；重跑时按真实依赖内容复用缓存，随后仍执行调用方的应用审计。
/// 编译任务受 Cargo jobserver 和 `SHADER_BUILD_JOBS` 上限共同约束；缓存及统计见模块文档。
pub fn build(
    recipes: &[Recipe<'_>],
    includes: &[&str],
    source_roots: &[&str],
) -> Result<(), Error> {
    build_inner(recipes, includes, source_roots, false).map(|_| ())
}

/// 构建实际产物，并在 `release-lto` 为 SPIR-V 另编同源 `-g3` 审计副本。
///
/// 返回 SPIR-V 审计目录；普通 profile 直接复用输出目录，WGSL 只写入生产目录。
/// 副本只位于 `OUT_DIR/audit`，不进入发布产物。
/// 调用方对副本执行源码归属审查，对发布产物校验接口及阶段不变量；不宣称两者执行图相同。
/// 两次编译的实际参数均进入构建身份和记录；审计副本另以 `AUDIT_SPIRV_BLAKE3` 标识。
/// # Errors
/// 与 [`build`] 相同；任一次编译失败均停止构建。
pub fn build_audited(
    recipes: &[Recipe<'_>],
    includes: &[&str],
    source_roots: &[&str],
) -> Result<PathBuf, Error> {
    build_inner(recipes, includes, source_roots, true)
}

fn build_inner(
    recipes: &[Recipe<'_>],
    includes: &[&str],
    source_roots: &[&str],
    audited: bool,
) -> Result<PathBuf, Error> {
    for recipe in recipes {
        validate_recipe(recipe)?;
    }
    let scheduler = batch::Scheduler::new()?;
    let compiler = Compiler::find(true)?;
    let output = env::var_os("OUT_DIR").ok_or(Error::OutputDirectory)?;
    let output = PathBuf::from(output);
    let mut roots: BTreeSet<PathBuf> = source_roots.iter().map(PathBuf::from).collect();
    roots.extend(includes.iter().map(PathBuf::from));
    roots.extend(recipes.iter().map(|recipe| {
        let source = Path::new(recipe.source);
        source
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(source)
            .to_owned()
    }));
    roots.extend([PathBuf::from("build.rs"), PathBuf::from("Cargo.toml")]);
    let tool = Path::new(env!("CARGO_MANIFEST_DIR"));
    roots.extend([
        tool.join("src"),
        tool.join("build.rs"),
        tool.join("Cargo.toml"),
    ]);
    for path in &roots {
        track(path);
    }
    let files = files(roots)?;
    let mut source_hash = blake3::Hasher::new();
    let mut build_hash = blake3::Hasher::new();
    field(&mut source_hash, compiler.identity.as_bytes());
    field(&mut build_hash, compiler.identity.as_bytes());
    for path in &files {
        let contents = read(path)?;
        let name = match path.strip_prefix(tool) {
            Ok(relative) => format!("@shader_build/{}", relative.display()),
            Err(_) => path.to_string_lossy().into_owned(),
        }
        .replace('\\', "/");
        field(&mut source_hash, name.as_bytes());
        field(&mut source_hash, &contents);
        if path
            .extension()
            .is_some_and(|extension| extension == "slang")
            || path == Path::new("build.rs")
            || path.starts_with(tool)
        {
            field(&mut build_hash, name.as_bytes());
            field(&mut build_hash, &contents);
        }
    }
    let mut spirv_hash = blake3::Hasher::new();
    let mut output_hash = blake3::Hasher::new();
    let mut audit_hash = blake3::Hasher::new();
    let mut manifest = format!(
        "compiler={}\nversion={}\ncompiler_blake3={}\nSLANG_RUN_SPIRV_VALIDATION=1\n",
        compiler.path.display(),
        compiler.version,
        compiler.identity,
    );
    let audit = audited && release_lto();
    let audit_output = if audit {
        output.join("audit")
    } else {
        output.clone()
    };
    if audit {
        fs::create_dir_all(&audit_output).map_err(|source| Error::Io {
            path: audit_output.clone(),
            source,
        })?;
    }
    field(
        &mut build_hash,
        if audit {
            b"audit=g3"
        } else {
            b"audit=production"
        },
    );
    let mut has_spirv = false;
    let mut tasks = Vec::new();
    for recipe in recipes {
        let destination = output.join(recipe.output);
        let mut arguments = vec![
            recipe.source.to_owned(),
            "-std".into(),
            "2026".into(),
            "-O3".into(),
            debug_argument(recipe.target).into(),
        ];
        for include in includes {
            arguments.extend(["-I".into(), (*include).into()]);
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
        for argument in &arguments {
            field(&mut build_hash, argument.as_bytes());
        }
        field(&mut build_hash, recipe.output.as_bytes());
        manifest.push_str(&format!(
            "{} {:?} -> {}\n",
            recipe.entry, arguments, recipe.output
        ));
        tasks.push(batch::Task::new(
            recipe.entry,
            arguments.clone(),
            destination,
        )?);
        if audit && matches!(recipe.target, Target::Spirv(_)) {
            // 审计反射同样留在副本目录，ABI 消费者只读取实际发布产物的反射。
            let reflection = audit_output.join(format!("{}.json", recipe.output));
            for index in 0..arguments.len() {
                if arguments[index] == "-reflection-json" {
                    if let Some(path) = arguments.get_mut(index + 1) {
                        *path = reflection.to_string_lossy().into_owned();
                    }
                } else if arguments[index].starts_with("-reflection-json=") {
                    arguments[index] = format!("-reflection-json={}", reflection.display());
                }
            }
            arguments.push("-g3".into());
            for argument in &arguments {
                field(&mut build_hash, argument.as_bytes());
            }
            let destination = audit_output.join(recipe.output);
            manifest.push_str(&format!(
                "audit {} {:?} -> {}\n",
                recipe.entry, arguments, recipe.output
            ));
            tasks.push(batch::Task::new(recipe.entry, arguments, destination)?);
        }
    }
    let statistics = batch::compile(&compiler, &scheduler, &output, &tasks, &files)?;
    manifest.push_str(&statistics);
    for recipe in recipes {
        let bytes = read(&output.join(recipe.output))?;
        field(&mut output_hash, recipe.output.as_bytes());
        field(&mut output_hash, &bytes);
        if matches!(recipe.target, Target::Spirv(_)) {
            has_spirv = true;
            field(&mut spirv_hash, recipe.entry.as_bytes());
            field(&mut spirv_hash, &bytes);
            if audit {
                field(&mut audit_hash, recipe.output.as_bytes());
                field(&mut audit_hash, &read(&audit_output.join(recipe.output))?);
            }
        }
    }
    if audit && has_spirv {
        emit("AUDIT_SPIRV_BLAKE3", &audit_hash.finalize().to_hex());
        manifest.push_str(&format!(
            "audit_spirv_blake3={}\n",
            audit_hash.finalize().to_hex()
        ));
    }
    emit("SLANG_VERSION", &compiler.version);
    emit("SLANG_COMPILER", &compiler.path.to_string_lossy());
    emit("SLANG_COMPILER_BLAKE3", &compiler.identity);
    emit("SOURCE_BLAKE3", &source_hash.finalize().to_hex());
    emit("SHADER_BUILD_BLAKE3", &build_hash.finalize().to_hex());
    emit("SHADER_OUTPUT_BLAKE3", &output_hash.finalize().to_hex());
    if has_spirv {
        emit("SPIRV_BLAKE3", &spirv_hash.finalize().to_hex());
    }
    let path = output.join("shader-build.txt");
    fs::write(&path, manifest).map_err(|source| Error::Io { path, source })?;
    Ok(audit_output)
}

struct Compiler {
    path: PathBuf,
    version: String,
    identity: String,
}

impl Compiler {
    fn command(&self) -> Command {
        validated_command(&self.path)
    }

    fn find(track_cargo: bool) -> Result<Self, Error> {
        let track = |path: &Path| {
            if track_cargo {
                crate::track(path);
            }
        };
        for variable in [
            "SLANGC",
            "VULKAN_SDK",
            "PATH",
            "PATHEXT",
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            if track_cargo {
                println!("cargo::rerun-if-env-changed={variable}");
            }
        }
        let name = if cfg!(windows) {
            "slangc.exe"
        } else {
            "slangc"
        };
        let requested = env::var_os("SLANGC")
            .map(PathBuf::from)
            .or_else(|| {
                let sdk = PathBuf::from(env::var_os("VULKAN_SDK")?);
                let path = sdk
                    .join(if cfg!(windows) { "Bin" } else { "bin" })
                    .join(name);
                if let Some(parent) = path.parent().filter(|parent| parent.is_dir()) {
                    track(parent);
                }
                path.is_file().then_some(path)
            })
            .unwrap_or_else(|| PathBuf::from(name));
        let resolved = if requested.components().count() > 1 || requested.is_absolute() {
            requested.clone()
        } else {
            let paths = env::var_os("PATH").unwrap_or_default();
            let names = if cfg!(windows) && requested.extension().is_none() {
                vec![requested.clone(), requested.with_extension("exe")]
            } else {
                vec![requested.clone()]
            };
            env::split_paths(&paths)
                .inspect(|directory| {
                    if directory.is_dir() {
                        track(directory);
                    }
                })
                .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
                .find(|path| path.is_file())
                .ok_or_else(|| Error::Compiler(requested.clone()))?
        };
        Self::inspect(&resolved, track)
    }

    fn inspect(resolved: &Path, track: impl Fn(&Path)) -> Result<Self, Error> {
        let path = fs::canonicalize(resolved).map_err(|source| Error::Io {
            path: resolved.to_owned(),
            source,
        })?;
        track(resolved);
        if let Some(parent) = resolved.parent().filter(|parent| parent.is_dir()) {
            track(parent);
        }
        track(&path);
        let parent = path.parent().expect("绝对编译器路径缺少父目录");
        // 安装目录跟踪组件增删；只将 Slang 组件纳入身份，避免无关 SDK 二进制影响指纹。
        let mut directories = BTreeSet::from([parent.to_owned()]);
        if let Some(root) = parent.parent() {
            let lib = root.join("lib");
            if lib.is_dir() {
                directories.insert(lib);
            }
        }
        for variable in [
            "LD_LIBRARY_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FALLBACK_LIBRARY_PATH",
        ] {
            if let Some(value) = env::var_os(variable) {
                directories.extend(env::split_paths(&value).filter(|directory| directory.is_dir()));
            }
        }
        let mut components = BTreeSet::from([path.clone()]);
        for directory in directories {
            track(&directory);
            for entry in fs::read_dir(&directory).map_err(|source| Error::Io {
                path: directory.clone(),
                source,
            })? {
                let entry = entry.map_err(|source| Error::Io {
                    path: directory.clone(),
                    source,
                })?;
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                // Slang 首次使用时写回派生 GLSL 模块；不让该缓存使下一次进程失去已有产物。
                if name != "slang-glsl-module.bin"
                    && (name.starts_with("slang") || name.starts_with("libslang"))
                {
                    components.insert(entry.path());
                }
            }
        }
        let version = run(validated_command(&path).arg("-version"), "-version", &path)?;
        let version = String::from_utf8_lossy(&version)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let mut hash = blake3::Hasher::new();
        field(&mut hash, path.to_string_lossy().as_bytes());
        field(&mut hash, version.as_bytes());
        field(&mut hash, b"SLANG_RUN_SPIRV_VALIDATION=1");
        for component in files(components)? {
            field(&mut hash, component.to_string_lossy().as_bytes());
            hash_file(&mut hash, &component)?;
        }
        Ok(Self {
            path,
            version,
            identity: hash.finalize().to_hex().to_string(),
        })
    }
}

// 不修改进程全局环境；每个编译子进程及编译器身份使用同一固定策略。
fn validated_command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.env("SLANG_RUN_SPIRV_VALIDATION", "1");
    command
}

fn validate_recipe(recipe: &Recipe<'_>) -> Result<(), Error> {
    if matches!(recipe.target, Target::Spirv(_))
        && recipe.args.iter().any(|argument| {
            argument.starts_with("-skip-spirv-validation")
                || argument.starts_with("-incomplete-library")
        })
    {
        return Err(Error::Validation);
    }
    Ok(())
}

fn run(command: &mut Command, entry: &str, compiler: &Path) -> Result<Vec<u8>, Error> {
    let result = command.output().map_err(|source| Error::Io {
        path: compiler.to_owned(),
        source,
    })?;
    if !result.status.success() {
        return Err(Error::Compile {
            compiler: compiler.to_owned(),
            entry: entry.to_owned(),
            diagnostic: format!(
                "{}\n{}\n{}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            ),
        });
    }
    Ok(if result.stdout.is_empty() {
        result.stderr
    } else {
        result.stdout
    })
}

fn files(roots: impl IntoIterator<Item = PathBuf>) -> Result<BTreeSet<PathBuf>, Error> {
    let mut pending: BTreeSet<_> = roots.into_iter().collect();
    let mut visited = BTreeSet::new();
    let mut result = BTreeSet::new();
    while let Some(path) = pending.pop_first() {
        let canonical = fs::canonicalize(&path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        if !visited.insert(canonical) {
            continue;
        }
        if path.is_dir() {
            for entry in fs::read_dir(&path).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })? {
                pending.insert(
                    entry
                        .map_err(|source| Error::Io {
                            path: path.clone(),
                            source,
                        })?
                        .path(),
                );
            }
        } else {
            result.insert(path);
        }
    }
    Ok(result)
}

fn read(path: &Path) -> Result<Vec<u8>, Error> {
    fs::read(path).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn hash_file(hash: &mut blake3::Hasher, path: &Path) -> Result<(), Error> {
    let result = (|| {
        let mut file = fs::File::open(path)?;
        hash.update(&file.metadata()?.len().to_le_bytes());
        hash.update_reader(&mut file)?;
        Ok::<_, io::Error>(())
    })();
    result.map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn field(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn track(path: &Path) {
    println!("cargo::rerun-if-changed={}", path.display());
}

fn emit(name: &str, value: &str) {
    println!("cargo::rustc-env={name}={value}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_process_fixes_the_validation_policy() {
        let command = validated_command(Path::new("slangc"));
        assert!(command.get_envs().any(|(key, value)| {
            key == "SLANG_RUN_SPIRV_VALIDATION" && value == Some(std::ffi::OsStr::new("1"))
        }));
    }

    #[test]
    fn spirv_recipe_cannot_skip_validation() {
        for argument in ["-skip-spirv-validation", "-incomplete-library"] {
            let recipe = Recipe {
                source: "unused.slang",
                entry: "main",
                stage: "compute",
                output: "main.spv",
                target: Target::Spirv("spirv_1_4"),
                args: &[argument],
            };
            assert!(matches!(validate_recipe(&recipe), Err(Error::Validation)));
        }
    }
}
