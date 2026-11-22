//! Cargo与资源准备共用的批量调度及依赖内容缓存；缓存字节不替代调用方的 ABI 和物理审计。
use super::{Compiler, Error, canonical_path, command_path, hash_field, read_file};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    thread,
    time::{Duration, Instant},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);
const CACHE_VERSION: &str = "slang-dependency-cache-2";

pub(super) struct Scheduler {
    jobs: usize,
    server: Option<jobserver::Client>,
}

impl Scheduler {
    pub fn new() -> Result<Self, Error> {
        for variable in ["SHADER_BUILD_JOBS", "SHADER_BUILD_CACHE"] {
            println!("cargo::rerun-if-env-changed={variable}");
        }
        // 仅从 Cargo 构建进程继承 jobserver；不接受任意 MAKEFLAGS 中的描述符。
        // Cargo 保证描述符在构建脚本期间活动；check_pipe 再检查 Unix pipe 类型。
        // Client 复制句柄并维持其所有权；不会把句柄继续传入 Slang 子进程。
        let server = if env::var_os("CARGO_MAKEFLAGS").is_some() {
            unsafe { jobserver::Client::from_env_ext(true) }.client.ok()
        } else {
            None
        };
        let jobs = match env::var("SHADER_BUILD_JOBS") {
            Ok(value) => value
                .parse::<usize>()
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| Error::Configuration("SHADER_BUILD_JOBS 须为正整数".into()))?,
            Err(env::VarError::NotPresent) => thread::available_parallelism()
                .map_or(1, usize::from)
                .min(8),
            Err(error) => return Err(Error::Configuration(error.to_string())),
        };
        Ok(Self { jobs, server })
    }
}

pub(super) struct Task {
    entry: String,
    arguments: Vec<String>,
    destination: PathBuf,
    reflection: Option<PathBuf>,
    pub audit: Option<PathBuf>,
    publish_output: bool,
}

impl Task {
    pub fn new(entry: &str, arguments: Vec<String>, destination: PathBuf) -> Result<Self, Error> {
        if arguments.first().is_none_or(String::is_empty) {
            return Err(Error::Configuration("缺少 shader 源文件".into()));
        }
        let mut reflection = None;
        for (i, argument) in arguments.iter().enumerate() {
            let path = if argument == "-reflection-json" {
                Some(
                    arguments
                        .get(i + 1)
                        .ok_or_else(|| Error::Configuration("-reflection-json 缺少路径".into()))?
                        .as_str(),
                )
            } else {
                argument.strip_prefix("-reflection-json=")
            };
            if let Some(path) = path
                && reflection.replace(absolute(Path::new(path))?).is_some()
            {
                return Err(Error::Configuration(
                    "每个入口只允许一个 reflection 输出".into(),
                ));
            }
            if argument == "-o"
                || argument == "-depfile"
                || argument.starts_with("-depfile=")
                || argument == "-separate-debug-info-output"
                || argument.starts_with("-separate-debug-info-output=")
            {
                return Err(Error::Configuration("-o/-depfile 由构建所有者管理".into()));
            }
        }
        let destination = absolute(&destination)?;
        let audit = arguments
            .iter()
            .any(|argument| argument == "-separate-debug-info")
            .then(|| destination.with_extension("audit.spv"));
        for path in std::iter::once(&destination).chain(reflection.iter()) {
            if path.file_name().is_none() {
                return Err(Error::Configuration(format!(
                    "输出缺少文件名：{}",
                    path.display()
                )));
            }
        }
        Ok(Self {
            entry: entry.into(),
            arguments,
            destination,
            reflection,
            audit,
            publish_output: true,
        })
    }

    pub fn cached(entry: &str, arguments: Vec<String>, fallback: PathBuf) -> Result<Self, Error> {
        let mut task = Self::new(entry, arguments, fallback)?;
        task.publish_output = false;
        Ok(task)
    }
}

/// 一次批量源码扫描、缓存核验及编译的实际统计；不含GPU管线创建。
#[derive(Clone, Copy, Debug)]
pub struct CompileStatistics {
    /// 已完成的入口数量，包含缓存命中。
    pub tasks: usize,
    /// 通过依赖及产物内容校验的命中数。
    pub cache_hits: usize,
    /// 本次调度采用的worker上限。
    pub workers: usize,
    /// 同时处理的任务峰值；缓存核验也属于任务。
    pub peak_tasks: usize,
    /// 源码清单、准备、调度和产物发布的墙钟时间。
    pub elapsed: Duration,
}

pub(super) struct Compiled {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    pub audit: Option<AuditFile>,
    pub fingerprint: String,
    pub cache_hit: bool,
}

pub(super) struct AuditFile {
    pub path: PathBuf,
    pub digest: String,
}

pub(super) struct Output {
    pub compiled: Vec<Compiled>,
    pub statistics: CompileStatistics,
    summary: String,
}

fn failure(path: &Path, source: io::Error) -> Error {
    Error::Io {
        path: path.to_owned(),
        source,
    }
}
fn absolute(path: &Path) -> Result<PathBuf, Error> {
    Ok(if path.is_absolute() {
        path.to_owned()
    } else {
        env::current_dir().map_err(|e| failure(path, e))?.join(path)
    })
}
struct InputSnapshot {
    output: PathBuf,
    package: Option<PathBuf>,
    roots: BTreeMap<PathBuf, Vec<(String, String)>>,
    hashes: BTreeMap<PathBuf, String>,
    emit_cargo: bool,
}

impl InputSnapshot {
    fn name(&self, path: &Path) -> String {
        match path.strip_prefix(&self.output) {
            Ok(path) => format!("@out/{}", path.display()),
            Err(_) => super::package_name(self.package.as_deref(), path)
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        }
        .replace('\\', "/")
    }
    fn resolve(&self, name: &str) -> PathBuf {
        if let Some(relative) = name.strip_prefix("@out/") {
            return self.output.join(relative);
        }
        if let (Some(package), Some(relative)) = (&self.package, name.strip_prefix("@package/")) {
            return package.join(relative);
        }
        PathBuf::from(name)
    }
    fn digest(&self, path: &Path) -> Result<Option<String>, Error> {
        match fs::read(path) {
            Ok(bytes) => Ok(Some(blake3::hash(&bytes).to_hex().to_string())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(failure(path, e)),
        }
    }
    fn inventory(&self, roots: &[PathBuf], extensions: &BTreeSet<String>) -> String {
        let mut hash = blake3::Hasher::new();
        for root in roots {
            hash_field(&mut hash, self.name(root).as_bytes());
            if let Some(paths) = self.roots.get(root) {
                for (ext, name) in paths {
                    if extensions.contains(ext) {
                        hash_field(&mut hash, name.as_bytes());
                    }
                }
            }
        }
        hash.finalize().to_hex().to_string()
    }
}
fn extension(path: &Path) -> String {
    path.extension()
        .map_or(String::new(), |e| e.to_string_lossy().to_ascii_lowercase())
}

#[derive(Serialize, Deserialize)]
struct Dependency {
    path: String,
    digest: String,
}
#[derive(Serialize, Deserialize)]
struct CacheManifest {
    version: String,
    dependencies: Vec<Dependency>,
    extensions: BTreeSet<String>,
    inventory: String,
    object: String,
    digest: String,
    reflection: Option<String>,
    audit: Option<String>,
}

struct PreparedTask<'a> {
    task: &'a Task,
    arguments: Vec<String>,
    roots: Vec<PathBuf>,
    key: String,
}

pub(super) fn compile(
    compiler: &Compiler,
    scheduler: &Scheduler,
    output: &Path,
    tasks: &[Task],
    tracked_files: &BTreeSet<PathBuf>,
) -> Result<String, Error> {
    let cache = match env::var_os("SHADER_BUILD_CACHE") {
        Some(path) => absolute(Path::new(&path))?,
        None => cache_root(output)?.join("shader-cache"),
    };
    compile_tracked(
        compiler,
        scheduler,
        output,
        tasks,
        &cache,
        tracked_files,
        true,
    )
    .map(|result| result.summary)
}

fn cache_root(output: &Path) -> Result<PathBuf, Error> {
    let parent = output
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| Error::Configuration("OUT_DIR 缺少 Cargo 包槽".into()))?;
    let build = if parent.file_name().is_some_and(|v| v == "build") {
        parent
    } else {
        parent
            .parent()
            .filter(|v| v.file_name().is_some_and(|v| v == "build"))
            .ok_or_else(|| Error::Configuration("OUT_DIR 缺少 Cargo build 目录".into()))?
    };
    let root = build
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| Error::Configuration("OUT_DIR 缺少 Cargo profile 目录".into()))?;
    Ok(if env::var_os("TARGET").as_deref() == root.file_name() {
        root.parent()
            .ok_or_else(|| Error::Configuration("OUT_DIR 缺少 target 根".into()))?
            .to_owned()
    } else {
        root.to_owned()
    })
}

#[cfg(test)]
fn compile_in(
    compiler: &Compiler,
    scheduler: &Scheduler,
    output: &Path,
    tasks: &[Task],
    cache: &Path,
) -> Result<String, Error> {
    compile_tracked(
        compiler,
        scheduler,
        output,
        tasks,
        cache,
        &BTreeSet::new(),
        true,
    )
    .map(|result| result.summary)
}

pub(super) fn compile_runtime(
    compiler: &Compiler,
    cache: &Path,
    tasks: &[Task],
    tracked_files: &BTreeSet<PathBuf>,
    jobs: std::num::NonZeroUsize,
) -> Result<Output, Error> {
    let scheduler = Scheduler {
        jobs: jobs.get(),
        server: None,
    };
    compile_tracked(
        compiler,
        &scheduler,
        cache,
        tasks,
        cache,
        tracked_files,
        false,
    )
}

// 准备阶段只验证并规范化请求；不启动worker或发布产物。
struct PreparedBatch<'a> {
    tasks: Vec<PreparedTask<'a>>,
    roots: BTreeSet<PathBuf>,
    outputs: BTreeSet<PathBuf>,
}

fn prepare_requests<'a>(
    compiler: &Compiler,
    output: &Path,
    tasks: &'a [Task],
) -> Result<PreparedBatch<'a>, Error> {
    let mut prepared = Vec::with_capacity(tasks.len());
    let mut all_roots = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    for task in tasks {
        for path in std::iter::once(&task.destination)
            .chain(task.reflection.iter())
            .chain(task.audit.iter())
        {
            let parent = canonical_path(
                path.parent()
                    .ok_or_else(|| Error::Configuration("输出缺少父目录".into()))?,
            )?;
            let path = parent.join(path.file_name().expect("输出有文件名"));
            if !outputs.insert(path.clone()) {
                return Err(Error::Configuration(format!("重复输出 {}", path.display())));
            }
        }
        let source = canonical_path(Path::new(&task.arguments[0]))?;
        if !source.is_file() {
            return Err(Error::Configuration(format!(
                "shader 源路径不是文件：{}",
                source.display()
            )));
        }
        let mut roots = vec![
            source
                .parent()
                .expect("canonical_path 文件父目录")
                .to_owned(),
        ];
        let mut args = task.arguments.clone();
        args[0] = normalize(&source, output, compiler.package.as_deref());
        let mut index = 1;
        while index < args.len() {
            if args[index] == "-I" {
                index += 1;
                let path = canonical_path(Path::new(
                    args.get(index)
                        .ok_or_else(|| Error::Configuration("-I 缺少路径".into()))?,
                ))?;
                roots.push(path.clone());
                args[index] = normalize(&path, output, compiler.package.as_deref());
            } else if args[index].starts_with("-I") && args[index].len() > 2 {
                let path = canonical_path(Path::new(&args[index][2..]))?;
                roots.push(path.clone());
                args[index] = format!(
                    "-I{}",
                    normalize(&path, output, compiler.package.as_deref())
                );
            } else if args[index] == "-reflection-json" {
                index += 1;
                args[index] = "@reflection".into();
            } else if args[index].starts_with("-reflection-json=") {
                args[index] = "-reflection-json=@reflection".into();
            }
            index += 1;
        }
        all_roots.extend(roots.iter().cloned());
        let mut hash = blake3::Hasher::new();
        if task.audit.is_some() {
            args.extend(["-separate-debug-info-output".into(), "@audit".into()]);
        }
        hash_field(&mut hash, CACHE_VERSION.as_bytes());
        hash_field(&mut hash, compiler.identity.as_bytes());
        // 带源码位置的产物及审计副本须指向本次提取目录。
        if let Some(package) = &compiler.package
            && args
                .iter()
                .rev()
                .find(|s| matches!(s.as_str(), "-g" | "-g0" | "-g1" | "-g2" | "-g3"))
                .is_none_or(|s| s != "-g0")
        {
            hash_field(&mut hash, package.to_string_lossy().as_bytes());
        }
        for argument in &args {
            hash_field(&mut hash, argument.as_bytes());
        }
        prepared.push(PreparedTask {
            task,
            arguments: args,
            roots,
            key: hash.finalize().to_hex().to_string(),
        });
    }
    Ok(PreparedBatch {
        tasks: prepared,
        roots: all_roots,
        outputs,
    })
}

fn snapshot_inputs(
    output: PathBuf,
    package: Option<PathBuf>,
    roots: BTreeSet<PathBuf>,
    outputs: &BTreeSet<PathBuf>,
    cache: &Path,
    tracked_files: &BTreeSet<PathBuf>,
    emit_cargo: bool,
) -> Result<InputSnapshot, Error> {
    let mut inputs = InputSnapshot {
        output,
        package,
        roots: BTreeMap::new(),
        hashes: BTreeMap::new(),
        emit_cargo,
    };
    for root in roots {
        let mut paths = Vec::new();
        let mut pending = vec![root.clone()];
        let mut visited = BTreeSet::new();
        while let Some(path) = pending.pop() {
            let path = canonical_path(&path)?;
            if path.starts_with(cache) || !visited.insert(path.clone()) {
                continue;
            }
            if path.is_dir() {
                for entry in fs::read_dir(&path).map_err(|e| failure(&path, e))? {
                    pending.push(entry.map_err(|e| failure(&path, e))?.path());
                }
            } else if !outputs.contains(&path) {
                inputs.hashes.entry(path.clone()).or_default();
                paths.push(path);
            }
        }
        paths.sort();
        let names = paths
            .iter()
            .map(|path| (extension(path), inputs.name(path)))
            .collect();
        inputs.roots.insert(root, names);
    }
    for path in tracked_files {
        let path = canonical_path(path)?;
        if !path.starts_with(cache) && !outputs.contains(&path) {
            inputs.hashes.entry(path).or_default();
        }
    }
    for (path, digest) in &mut inputs.hashes {
        *digest = blake3::hash(&read_file(path)?).to_hex().to_string();
    }
    Ok(inputs)
}

fn compile_tracked(
    compiler: &Compiler,
    scheduler: &Scheduler,
    output: &Path,
    tasks: &[Task],
    cache: &Path,
    tracked_files: &BTreeSet<PathBuf>,
    emit_cargo: bool,
) -> Result<Output, Error> {
    let started = Instant::now();
    let output = canonical_path(output)?;
    fs::create_dir_all(cache.join("requests")).map_err(|e| failure(cache, e))?;
    fs::create_dir_all(cache.join("objects")).map_err(|e| failure(cache, e))?;
    fs::create_dir_all(cache.join("temporary")).map_err(|e| failure(cache, e))?;
    let cache = canonical_path(cache)?;
    let PreparedBatch {
        tasks: prepared,
        roots,
        outputs,
    } = prepare_requests(compiler, &output, tasks)?;
    let inputs = snapshot_inputs(
        output,
        compiler.package.clone(),
        roots,
        &outputs,
        &cache,
        tracked_files,
        emit_cargo,
    )?;
    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    let jobs = scheduler.jobs.min(tasks.len().max(1));
    let mut results: Vec<Vec<(usize, Result<Compiled, Error>)>> =
        (0..jobs).map(|_| Vec::new()).collect();
    thread::scope(|scope| -> Result<(), Error> {
        let mut handles = Vec::new();
        let mut worker_failure = None;
        let mut worker_panic = None;
        for (worker, results) in results.iter_mut().enumerate() {
            let (next, failed, active, peak, inputs, prepared, cache) =
                (&next, &failed, &active, &peak, &inputs, &prepared, &cache);
            let spawned = thread::Builder::new().spawn_scoped(scope, move || {
                while !failed.load(Ordering::Acquire)
                    && next.load(Ordering::Acquire) < prepared.len()
                {
                    let token =
                        if let Some(server) = scheduler.server.as_ref().filter(|_| worker != 0) {
                            match server.try_acquire() {
                                Ok(Some(token)) => Some(token),
                                Ok(None) => {
                                    thread::sleep(Duration::from_millis(10));
                                    continue;
                                }
                                Err(e) => {
                                    results.push((
                                        usize::MAX,
                                        Err(failure(Path::new("Cargo jobserver"), e)),
                                    ));
                                    failed.store(true, Ordering::Release);
                                    break;
                                }
                            }
                        } else {
                            None
                        };
                    let at = next.fetch_add(1, Ordering::AcqRel);
                    let Some(task) = prepared.get(at) else {
                        break;
                    };
                    let count = active.fetch_add(1, Ordering::AcqRel) + 1;
                    peak.fetch_max(count, Ordering::AcqRel);
                    let result = execute_request(compiler, inputs, cache, task);
                    active.fetch_sub(1, Ordering::AcqRel);
                    if result.is_err() {
                        failed.store(true, Ordering::Release);
                    }
                    results.push((at, result));
                    drop(token);
                }
            });
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(error) => {
                    failed.store(true, Ordering::Release);
                    worker_failure = Some(Error::Io {
                        path: PathBuf::from("shader worker"),
                        source: error,
                    });
                    break;
                }
            }
        }
        for handle in handles {
            if let Err(payload) = handle.join() {
                failed.store(true, Ordering::Release);
                worker_panic.get_or_insert(payload);
            }
        }
        if let Some(payload) = worker_panic {
            std::panic::resume_unwind(payload);
        }
        worker_failure.map_or(Ok(()), Err)
    })?;
    let mut hits = 0;
    let mut compiled: Vec<Option<Compiled>> = (0..tasks.len()).map(|_| None).collect();
    for (index, result) in results.into_iter().flatten() {
        let result = result?;
        hits += usize::from(result.cache_hit);
        compiled[index] = Some(result);
    }
    let compiled = compiled
        .into_iter()
        .map(|v| v.expect("全部 worker 正常结束且任务均成功时结果完整"))
        .collect::<Vec<_>>();
    let count = compiled.len();
    let summary = format!(
        "cache={}\ntasks={count}\ncache_hits={hits}\ncompiled={}\nworkers={jobs}\npeak_tasks={}\nelapsed_ms={}\n",
        command_path(&cache),
        count - hits,
        peak.load(Ordering::Acquire),
        started.elapsed().as_millis()
    );
    if emit_cargo {
        println!(
            "cargo::warning=Slang: {hits}/{count} cache hits, {} compiled, {} ms, peak {}",
            count - hits,
            started.elapsed().as_millis(),
            peak.load(Ordering::Acquire)
        );
    }
    Ok(Output {
        compiled,
        summary,
        statistics: CompileStatistics {
            tasks: count,
            cache_hits: hits,
            workers: jobs,
            peak_tasks: peak.load(Ordering::Acquire),
            elapsed: started.elapsed(),
        },
    })
}

fn normalize(path: &Path, output: &Path, package: Option<&Path>) -> String {
    path.strip_prefix(output)
        .map_or_else(
            |_| super::package_name(package, path).unwrap_or_else(|| command_path(path)),
            |p| format!("@out/{}", p.display()),
        )
        .replace('\\', "/")
}

fn execute_request(
    compiler: &Compiler,
    inputs: &InputSnapshot,
    cache: &Path,
    prepared: &PreparedTask<'_>,
) -> Result<Compiled, Error> {
    let index = cache
        .join("requests")
        .join(format!("{}.json", prepared.key));
    let lock_path = index.with_extension("lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| failure(&lock_path, e))?;
    lock.lock().map_err(|e| failure(&lock_path, e))?;
    let current = load_manifest(&index)?;
    if let Some(manifest) = &current
        && let Some(result) = restore(inputs, cache, prepared, manifest)?
    {
        return Ok(result);
    }
    let history = cache.join("requests").join(&prepared.key);
    // 先保存旧单索引，兼容已有缓存。热命中不枚举历史目录。
    if let Some(manifest) = &current {
        archive_manifest(&history, &prepared.key, manifest)?;
    }
    let candidates = match fs::read_dir(&history) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|e| failure(&history, e))?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeSet::new(),
        Err(e) => return Err(failure(&history, e)),
    };
    for path in candidates {
        if path.extension().is_none_or(|v| v != "json")
            || current.as_ref().is_some_and(|v| {
                path.file_stem()
                    .is_some_and(|stem| stem == v.object.as_str())
            })
        {
            continue;
        }
        if let Some(manifest) = load_manifest(&path)?
            && let Some(result) = restore(inputs, cache, prepared, &manifest)?
        {
            publish_file(&index, &encode_manifest(&manifest)?)?;
            return Ok(result);
        }
    }
    compile_request(compiler, inputs, cache, prepared, &index, &history)
}

fn compile_request(
    compiler: &Compiler,
    inputs: &InputSnapshot,
    cache: &Path,
    prepared: &PreparedTask<'_>,
    index: &Path,
    history: &Path,
) -> Result<Compiled, Error> {
    let temporary = TemporaryDirectory::new(&cache.join("temporary"))?;
    let destination = temporary.path.join("shader");
    let reflection = temporary.path.join("reflection.json");
    let audit = temporary.path.join("audit.spv");
    let depfile = temporary.path.join("dependencies.d");
    let mut args = prepared.task.arguments.clone();
    for index in 0..args.len() {
        if args[index] == "-reflection-json" {
            args[index + 1] = command_path(&reflection);
        } else if args[index].starts_with("-reflection-json=") {
            args[index] = format!("-reflection-json={}", command_path(&reflection));
        }
    }
    let mut command = compiler.command();
    if prepared.task.audit.is_some() {
        args.extend(["-separate-debug-info-output".into(), command_path(&audit)]);
    }
    command
        .args(&args)
        .arg("-depfile")
        .arg(command_path(&depfile))
        .arg("-o")
        .arg(command_path(&destination));
    let result = command.output().map_err(|e| failure(&compiler.path, e))?;
    let diagnostic = format!(
        "{}\n{}\n{}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    if !result.status.success() {
        return Err(Error::Compile {
            compiler: compiler.path.clone(),
            entry: prepared.task.entry.clone(),
            diagnostic,
        });
    }
    let paths = dependencies(&read_file(&depfile)?)?;
    let mut deps = Vec::new();
    let mut cacheable = true;
    let mut extensions = BTreeSet::from(["slang".into(), "slang-module".into()]);
    for path in paths {
        let path = canonical_path(&path)?;
        let digest = inputs
            .digest(&path)?
            .ok_or_else(|| Error::InputChanged(path.clone()))?;
        // 陌生扩展的 include 也在预备快照内；越出搜索根的依赖不缓存，避免编译期间竞态。
        match inputs.hashes.get(&path) {
            Some(before) if before != &digest => return Err(Error::InputChanged(path)),
            Some(_) => {}
            None => {
                cacheable = false;
                if inputs.emit_cargo {
                    println!("cargo::rerun-if-changed={}", path.display());
                }
            }
        }
        extensions.insert(extension(&path));
        deps.push(Dependency {
            path: inputs.name(&path),
            digest,
        });
    }
    deps.sort_by(|a, b| a.path.cmp(&b.path));
    let bytes = read_file(&destination)?;
    let audit_bytes = prepared
        .task
        .audit
        .as_ref()
        .map(|_| read_file(&audit))
        .transpose()?;
    if let Some(audit_bytes) = &audit_bytes {
        super::verify_audit_pair(&bytes, audit_bytes)?;
    }
    let reflection_bytes = prepared
        .task
        .reflection
        .as_ref()
        .map(|_| read_file(&reflection))
        .transpose()?;
    let mut manifest = CacheManifest {
        version: CACHE_VERSION.into(),
        dependencies: deps,
        extensions: extensions.clone(),
        inventory: inputs.inventory(&prepared.roots, &extensions),
        object: String::new(),
        digest: blake3::hash(&bytes).to_hex().to_string(),
        reflection: reflection_bytes
            .as_ref()
            .map(|b| blake3::hash(b).to_hex().to_string()),
        audit: audit_bytes
            .as_ref()
            .map(|bytes| blake3::hash(bytes).to_hex().to_string()),
    };
    manifest.object = object_identity(&prepared.key, &manifest);
    let object = manifest.object.clone();
    if !cacheable {
        let diagnostic = format!(
            "compiler={}\narguments={:?}\nmain_blake3={}\naudit_blake3={:?}\n{diagnostic}",
            compiler.path.display(),
            command.get_args().collect::<Vec<_>>(),
            manifest.digest,
            manifest.audit
        );
        publish_task(
            prepared.task,
            &bytes,
            audit_bytes.as_deref(),
            reflection_bytes.as_deref(),
            true,
            Some(diagnostic.as_bytes()),
        )?;
        if inputs.emit_cargo {
            println!(
                "cargo::warning=Slang {} 包含未声明搜索根或 source_roots 的依赖，本次不缓存",
                prepared.task.entry
            );
        }
        return Ok(Compiled {
            path: prepared.task.destination.clone(),
            bytes,
            audit: manifest.audit.map(|digest| AuditFile {
                path: prepared
                    .task
                    .audit
                    .clone()
                    .expect("审计manifest对应请求目标"),
                digest,
            }),
            fingerprint: object,
            cache_hit: false,
        });
    }
    fs::write(
        temporary.path.join("diagnostic.txt"),
        format!(
            "compiler={}\narguments={:?}\nnormalized_arguments={:?}\nmain_blake3={}\naudit_blake3={:?}\n{diagnostic}",
            compiler.path.display(),
            command.get_args().collect::<Vec<_>>(),
            prepared.arguments,
            manifest.digest,
            manifest.audit
        ),
    )
    .map_err(|e| failure(&temporary.path, e))?;
    let object_dir = cache.join("objects").join(&object);
    // 请求锁串行化同一输入；校验失败的旧目录只保存再生数据，逐文件替换后才发布索引。
    fs::create_dir_all(&object_dir).map_err(|e| failure(&object_dir, e))?;
    let main_path = object_dir.join("shader");
    let audit_path = object_dir.join("audit.spv");
    let reflection_path = object_dir.join("reflection.json");
    let diagnostic_path = object_dir.join("diagnostic.txt");
    let diagnostic_bytes = read_file(&temporary.path.join("diagnostic.txt"))?;
    let mut object_files = vec![
        (main_path.as_path(), bytes.as_slice()),
        (diagnostic_path.as_path(), diagnostic_bytes.as_slice()),
    ];
    if let Some(audit) = &audit_bytes {
        object_files.push((&audit_path, audit));
    }
    if let Some(reflection) = &reflection_bytes {
        object_files.push((&reflection_path, reflection));
    }
    publish_files(&object_files)?;
    publish_task(
        prepared.task,
        &bytes,
        audit_bytes.as_deref(),
        reflection_bytes.as_deref(),
        false,
        None,
    )?;
    archive_manifest(history, &prepared.key, &manifest)?;
    publish_file(index, &encode_manifest(&manifest)?)?;
    Ok(Compiled {
        path: object_dir.join("shader"),
        bytes,
        audit: manifest.audit.map(|digest| AuditFile {
            path: audit_path,
            digest,
        }),
        fingerprint: object,
        cache_hit: false,
    })
}

fn encode_manifest(manifest: &CacheManifest) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(manifest).map_err(|e| Error::Configuration(e.to_string()))
}

fn archive_manifest(directory: &Path, key: &str, manifest: &CacheManifest) -> Result<(), Error> {
    if manifest.version == CACHE_VERSION && object_identity(key, manifest) == manifest.object {
        fs::create_dir_all(directory).map_err(|e| failure(directory, e))?;
        publish_file(
            &directory.join(format!("{}.json", manifest.object)),
            &encode_manifest(manifest)?,
        )?;
    }
    Ok(())
}

fn restore(
    inputs: &InputSnapshot,
    cache: &Path,
    prepared: &PreparedTask<'_>,
    manifest: &CacheManifest,
) -> Result<Option<Compiled>, Error> {
    if manifest.version != CACHE_VERSION
        || object_identity(&prepared.key, manifest) != manifest.object
        || prepared.task.reflection.is_some() != manifest.reflection.is_some()
        || prepared.task.audit.is_some() != manifest.audit.is_some()
        || manifest.inventory != inputs.inventory(&prepared.roots, &manifest.extensions)
    {
        return Ok(None);
    }
    for dep in &manifest.dependencies {
        let path = inputs.resolve(&dep.path);
        let matches = match inputs.hashes.get(&path) {
            Some(digest) => digest == &dep.digest,
            None => inputs.digest(&path)?.as_ref() == Some(&dep.digest),
        };
        if !matches {
            return Ok(None);
        }
    }
    let object = cache.join("objects").join(&manifest.object);
    let Some(bytes) = read_verified_file(&object.join("shader"), &manifest.digest)? else {
        return Ok(None);
    };
    let audit = match &manifest.audit {
        Some(hash) => match read_verified_file(&object.join("audit.spv"), hash)? {
            Some(audit) => {
                if super::verify_audit_pair(&bytes, &audit).is_err() {
                    return Ok(None);
                }
                Some(audit)
            }
            None => return Ok(None),
        },
        None => None,
    };
    let reflection = match &manifest.reflection {
        Some(hash) => read_verified_file(&object.join("reflection.json"), hash)?,
        None => Some(Vec::new()),
    };
    let Some(reflection) = reflection else {
        return Ok(None);
    };
    publish_task(
        prepared.task,
        &bytes,
        audit.as_deref(),
        Some(&reflection),
        false,
        None,
    )?;
    Ok(Some(Compiled {
        path: object.join("shader"),
        bytes,
        audit: manifest.audit.as_ref().map(|digest| AuditFile {
            path: object.join("audit.spv"),
            digest: digest.clone(),
        }),
        fingerprint: manifest.object.clone(),
        cache_hit: true,
    }))
}

fn object_identity(key: &str, manifest: &CacheManifest) -> String {
    let mut hash = blake3::Hasher::new();
    hash_field(&mut hash, key.as_bytes());
    hash_field(&mut hash, manifest.version.as_bytes());
    for dep in &manifest.dependencies {
        hash_field(&mut hash, dep.path.as_bytes());
        hash_field(&mut hash, dep.digest.as_bytes());
    }
    for extension in &manifest.extensions {
        hash_field(&mut hash, extension.as_bytes());
    }
    hash_field(&mut hash, manifest.inventory.as_bytes());
    hash_field(&mut hash, manifest.digest.as_bytes());
    if let Some(reflection) = &manifest.reflection {
        hash_field(&mut hash, reflection.as_bytes());
    }
    if let Some(audit) = &manifest.audit {
        hash_field(&mut hash, b"paired-audit");
        hash_field(&mut hash, audit.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}

fn load_manifest(path: &Path) -> Result<Option<CacheManifest>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(failure(path, e)),
    }
}
fn read_verified_file(path: &Path, hash: &str) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok((blake3::hash(&bytes).to_hex().as_str() == hash).then_some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(failure(path, e)),
    }
}
pub(super) fn publish_file(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if fs::read(path).ok().is_some_and(|old| old == bytes) {
        return Ok(());
    }
    let temporary = path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&temporary, bytes).map_err(|e| failure(&temporary, e))?;
    fs::rename(&temporary, path).map_err(|e| failure(path, e))
}

fn publish_task(
    task: &Task,
    bytes: &[u8],
    audit: Option<&[u8]>,
    reflection: Option<&[u8]>,
    uncached: bool,
    diagnostic: Option<&[u8]>,
) -> Result<(), Error> {
    let mut files = Vec::new();
    if task.publish_output || uncached {
        files.push((task.destination.as_path(), bytes));
        if let Some(path) = &task.audit {
            files.push((path.as_path(), audit.expect("已验证配对任务拥有副本")));
        }
    }
    if let Some(path) = &task.reflection {
        files.push((path.as_path(), reflection.expect("已验证反射任务拥有反射")));
    }
    let diagnostic_path = task.destination.with_extension("txt");
    if let Some(bytes) = diagnostic {
        files.push((&diagnostic_path, bytes));
    }
    publish_files(&files)
}

struct PendingFile<'a> {
    path: &'a Path,
    temporary: PathBuf,
    previous: Option<Vec<u8>>,
}

impl Drop for PendingFile<'_> {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

// 所有文件先完成暂存，再在请求锁内发布；失败恢复已替换文件，索引始终最后提交。
// 调用方只在请求成功后消费整组输出；不为绕过owner并发读取裸路径提供跨文件事务。
fn publish_files(files: &[(&Path, &[u8])]) -> Result<(), Error> {
    let mut pending = Vec::new();
    for &(path, bytes) in files {
        let previous = match fs::read(path) {
            Ok(old) if old == bytes => continue,
            Ok(old) => Some(old),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(failure(path, error)),
        };
        let temporary = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        pending.push(PendingFile {
            path,
            temporary,
            previous,
        });
        let file = pending.last().expect("刚登记暂存文件");
        fs::write(&file.temporary, bytes).map_err(|error| failure(&file.temporary, error))?;
    }
    for (index, file) in pending.iter().enumerate() {
        if let Err(source) = fs::rename(&file.temporary, file.path) {
            let mut rollback = None;
            for old in pending[..index].iter().rev() {
                let result = if let Some(bytes) = &old.previous {
                    fs::write(&old.temporary, bytes)
                        .and_then(|()| fs::rename(&old.temporary, old.path))
                } else {
                    fs::remove_file(old.path)
                };
                if let Err(error) = result {
                    rollback.get_or_insert(error);
                }
            }
            return Err(Error::Publication {
                path: file.path.to_owned(),
                source,
                rollback,
            });
        }
    }
    Ok(())
}
struct TemporaryDirectory {
    path: PathBuf,
}
impl TemporaryDirectory {
    fn new(parent: &Path) -> Result<Self, Error> {
        loop {
            let path = parent.join(format!(
                "{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(failure(&path, e)),
            }
        }
    }
}
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// Slang depfile 使用 Make 转义；分隔符是冒号后空白，不能误切 Windows 驱动器冒号。
fn dependencies(bytes: &[u8]) -> Result<Vec<PathBuf>, Error> {
    let text = String::from_utf8_lossy(bytes)
        .replace("\\\r\n", "")
        .replace("\\\n", "");
    let start = text
        .char_indices()
        .find(|&(i, c)| c == ':' && text[i + 1..].starts_with(char::is_whitespace))
        .map(|(i, _)| i + 1)
        .ok_or_else(|| Error::Configuration("Slang depfile 缺少目标分隔符".into()))?;
    let mut values = Vec::new();
    let mut word = String::new();
    let mut chars = text[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .peek()
                .is_some_and(|c| c.is_whitespace() || matches!(c, '\\' | '#' | ':'))
        {
            word.push(chars.next().expect("已检查转义字符"));
        } else if c.is_whitespace() {
            if !word.is_empty() {
                values.push(PathBuf::from(std::mem::take(&mut word)));
            }
        } else if c == '$' && chars.peek() == Some(&'$') {
            chars.next();
            word.push('$');
        } else {
            word.push(c);
        }
    }
    if !word.is_empty() {
        values.push(PathBuf::from(word));
    }
    Ok(values)
}

#[cfg(test)]
#[path = "batch_tests.rs"]
mod tests;
