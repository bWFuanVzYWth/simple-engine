//! Cargo与资源准备共用的批量调度及依赖内容缓存；缓存字节不替代调用方的 ABI 和物理审计。
use super::{Compiler, Error, field, read};
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
const VERSION: &str = "slang-dependency-cache-1";

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
            if argument == "-o" || argument == "-depfile" || argument.starts_with("-depfile=") {
                return Err(Error::Configuration("-o/-depfile 由构建所有者管理".into()));
            }
        }
        let destination = absolute(&destination)?;
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
    pub fingerprint: String,
    pub cache_hit: bool,
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
fn canonical(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|e| failure(path, e))
}
fn command_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        if let Some(path) = text.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }
        if let Some(path) = text.strip_prefix(r"\\?\") {
            return path.into();
        }
    }
    text.into_owned()
}

struct Inputs {
    output: PathBuf,
    roots: BTreeMap<PathBuf, Vec<PathBuf>>,
    hashes: BTreeMap<PathBuf, String>,
    cargo: bool,
}

impl Inputs {
    fn name(&self, path: &Path) -> String {
        match path.strip_prefix(&self.output) {
            Ok(path) => format!("@out/{}", path.display()),
            Err(_) => path.to_string_lossy().into_owned(),
        }
        .replace('\\', "/")
    }
    fn resolve(&self, name: &str) -> PathBuf {
        name.strip_prefix("@out/")
            .map_or_else(|| PathBuf::from(name), |name| self.output.join(name))
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
            field(&mut hash, self.name(root).as_bytes());
            if let Some(paths) = self.roots.get(root) {
                for path in paths {
                    if extensions.contains(&extension(path)) {
                        field(&mut hash, self.name(path).as_bytes());
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
struct Manifest {
    version: String,
    dependencies: Vec<Dependency>,
    extensions: BTreeSet<String>,
    inventory: String,
    object: String,
    digest: String,
    reflection: Option<String>,
}

struct Prepared<'a> {
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

fn compile_tracked(
    compiler: &Compiler,
    scheduler: &Scheduler,
    output: &Path,
    tasks: &[Task],
    cache: &Path,
    tracked_files: &BTreeSet<PathBuf>,
    cargo: bool,
) -> Result<Output, Error> {
    let started = Instant::now();
    let output = canonical(output)?;
    fs::create_dir_all(cache.join("requests")).map_err(|e| failure(cache, e))?;
    fs::create_dir_all(cache.join("objects")).map_err(|e| failure(cache, e))?;
    fs::create_dir_all(cache.join("temporary")).map_err(|e| failure(cache, e))?;
    let cache = canonical(cache)?;
    let mut prepared = Vec::with_capacity(tasks.len());
    let mut all_roots = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    for task in tasks {
        for path in std::iter::once(&task.destination).chain(task.reflection.iter()) {
            let parent = canonical(
                path.parent()
                    .ok_or_else(|| Error::Configuration("输出缺少父目录".into()))?,
            )?;
            let path = parent.join(path.file_name().expect("输出有文件名"));
            if !outputs.insert(path.clone()) {
                return Err(Error::Configuration(format!("重复输出 {}", path.display())));
            }
        }
        let source = canonical(Path::new(&task.arguments[0]))?;
        if !source.is_file() {
            return Err(Error::Configuration(format!(
                "shader 源路径不是文件：{}",
                source.display()
            )));
        }
        let mut roots = vec![source.parent().expect("canonical 文件父目录").to_owned()];
        let mut args = task.arguments.clone();
        args[0] = normalize(&source, &output);
        let mut index = 1;
        while index < args.len() {
            if args[index] == "-I" {
                index += 1;
                let path = canonical(Path::new(
                    args.get(index)
                        .ok_or_else(|| Error::Configuration("-I 缺少路径".into()))?,
                ))?;
                roots.push(path.clone());
                args[index] = normalize(&path, &output);
            } else if args[index].starts_with("-I") && args[index].len() > 2 {
                let path = canonical(Path::new(&args[index][2..]))?;
                roots.push(path.clone());
                args[index] = format!("-I{}", normalize(&path, &output));
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
        field(&mut hash, VERSION.as_bytes());
        field(&mut hash, compiler.identity.as_bytes());
        for argument in &args {
            field(&mut hash, argument.as_bytes());
        }
        prepared.push(Prepared {
            task,
            arguments: args,
            roots,
            key: hash.finalize().to_hex().to_string(),
        });
    }
    let mut inputs = Inputs {
        output,
        roots: BTreeMap::new(),
        hashes: BTreeMap::new(),
        cargo,
    };
    for root in all_roots {
        let mut paths = Vec::new();
        let mut pending = vec![root.clone()];
        let mut visited = BTreeSet::new();
        while let Some(path) = pending.pop() {
            let path = canonical(&path)?;
            if path.starts_with(&cache) || !visited.insert(path.clone()) {
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
        inputs.roots.insert(root, paths);
    }
    for path in tracked_files {
        let path = canonical(path)?;
        if !path.starts_with(&cache) && !outputs.contains(&path) {
            inputs.hashes.entry(path).or_default();
        }
    }
    for (path, digest) in &mut inputs.hashes {
        *digest = blake3::hash(&read(path)?).to_hex().to_string();
    }
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
                    let result = execute(compiler, inputs, cache, task);
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
    if cargo {
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

fn normalize(path: &Path, output: &Path) -> String {
    path.strip_prefix(output)
        .map_or_else(|_| command_path(path), |p| format!("@out/{}", p.display()))
        .replace('\\', "/")
}

fn execute(
    compiler: &Compiler,
    inputs: &Inputs,
    cache: &Path,
    prepared: &Prepared<'_>,
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
    let current = load(&index)?;
    if let Some(manifest) = &current
        && let Some(result) = restore(inputs, cache, prepared, manifest)?
    {
        return Ok(result);
    }
    let history = cache.join("requests").join(&prepared.key);
    // 先保存旧单索引，兼容已有缓存。热命中不枚举历史目录。
    if let Some(manifest) = &current {
        remember(&history, &prepared.key, manifest)?;
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
        if let Some(manifest) = load(&path)?
            && let Some(result) = restore(inputs, cache, prepared, &manifest)?
        {
            publish(&index, &encode(&manifest)?)?;
            return Ok(result);
        }
    }
    let temporary = Temporary::new(&cache.join("temporary"))?;
    let destination = temporary.0.join("shader");
    let reflection = temporary.0.join("reflection.json");
    let depfile = temporary.0.join("dependencies.d");
    let mut args = prepared.task.arguments.clone();
    for index in 0..args.len() {
        if args[index] == "-reflection-json" {
            args[index + 1] = command_path(&reflection);
        } else if args[index].starts_with("-reflection-json=") {
            args[index] = format!("-reflection-json={}", command_path(&reflection));
        }
    }
    let mut command = compiler.command();
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
    let paths = dependencies(&read(&depfile)?)?;
    let mut deps = Vec::new();
    let mut cacheable = true;
    let mut extensions = BTreeSet::from(["slang".into(), "slang-module".into()]);
    for path in paths {
        let path = canonical(&path)?;
        let digest = inputs
            .digest(&path)?
            .ok_or_else(|| Error::InputChanged(path.clone()))?;
        // 陌生扩展的 include 也在预备快照内；越出搜索根的依赖不缓存，避免编译期间竞态。
        match inputs.hashes.get(&path) {
            Some(before) if before != &digest => return Err(Error::InputChanged(path)),
            Some(_) => {}
            None => {
                cacheable = false;
                if inputs.cargo {
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
    let bytes = read(&destination)?;
    let reflection_bytes = prepared
        .task
        .reflection
        .as_ref()
        .map(|_| read(&reflection))
        .transpose()?;
    let mut manifest = Manifest {
        version: VERSION.into(),
        dependencies: deps,
        extensions: extensions.clone(),
        inventory: inputs.inventory(&prepared.roots, &extensions),
        object: String::new(),
        digest: blake3::hash(&bytes).to_hex().to_string(),
        reflection: reflection_bytes
            .as_ref()
            .map(|b| blake3::hash(b).to_hex().to_string()),
    };
    manifest.object = object_identity(&prepared.key, &manifest);
    let object = manifest.object.clone();
    if !cacheable {
        publish(&prepared.task.destination, &bytes)?;
        if let (Some(path), Some(bytes)) = (&prepared.task.reflection, &reflection_bytes) {
            publish(path, bytes)?;
        }
        if inputs.cargo {
            println!(
                "cargo::warning=Slang {} 包含未声明搜索根或 source_roots 的依赖，本次不缓存",
                prepared.task.entry
            );
        }
        publish(
            &prepared.task.destination.with_extension("txt"),
            diagnostic.as_bytes(),
        )?;
        return Ok(Compiled {
            path: prepared.task.destination.clone(),
            fingerprint: object,
            cache_hit: false,
        });
    }
    fs::write(
        temporary.0.join("diagnostic.txt"),
        format!(
            "compiler={}\narguments={:?}\nnormalized_arguments={:?}\n{diagnostic}",
            compiler.path.display(),
            command.get_args().collect::<Vec<_>>(),
            prepared.arguments
        ),
    )
    .map_err(|e| failure(&temporary.0, e))?;
    let object_dir = cache.join("objects").join(&object);
    // 请求锁串行化同一输入；校验失败的旧目录只保存再生数据，逐文件替换后才发布索引。
    fs::create_dir_all(&object_dir).map_err(|e| failure(&object_dir, e))?;
    publish(&object_dir.join("shader"), &bytes)?;
    if let Some(reflection) = &reflection_bytes {
        publish(&object_dir.join("reflection.json"), reflection)?;
    }
    publish(
        &object_dir.join("diagnostic.txt"),
        &read(&temporary.0.join("diagnostic.txt"))?,
    )?;
    remember(&history, &prepared.key, &manifest)?;
    publish(&index, &encode(&manifest)?)?;
    if prepared.task.publish_output {
        publish(&prepared.task.destination, &bytes)?;
    }
    if let (Some(path), Some(bytes)) = (&prepared.task.reflection, reflection_bytes) {
        publish(path, &bytes)?;
    }
    Ok(Compiled {
        path: object_dir.join("shader"),
        fingerprint: object,
        cache_hit: false,
    })
}

fn encode(manifest: &Manifest) -> Result<Vec<u8>, Error> {
    serde_json::to_vec(manifest).map_err(|e| Error::Configuration(e.to_string()))
}

fn remember(directory: &Path, key: &str, manifest: &Manifest) -> Result<(), Error> {
    if manifest.version == VERSION && object_identity(key, manifest) == manifest.object {
        fs::create_dir_all(directory).map_err(|e| failure(directory, e))?;
        publish(
            &directory.join(format!("{}.json", manifest.object)),
            &encode(manifest)?,
        )?;
    }
    Ok(())
}

fn restore(
    inputs: &Inputs,
    cache: &Path,
    prepared: &Prepared<'_>,
    manifest: &Manifest,
) -> Result<Option<Compiled>, Error> {
    if manifest.version != VERSION
        || object_identity(&prepared.key, manifest) != manifest.object
        || prepared.task.reflection.is_some() != manifest.reflection.is_some()
        || manifest.inventory != inputs.inventory(&prepared.roots, &manifest.extensions)
    {
        return Ok(None);
    }
    for dep in &manifest.dependencies {
        let path = inputs.resolve(&dep.path);
        let digest = match inputs.hashes.get(&path) {
            Some(digest) => Some(digest.clone()),
            None => inputs.digest(&path)?,
        };
        if digest.as_ref() != Some(&dep.digest) {
            return Ok(None);
        }
    }
    let object = cache.join("objects").join(&manifest.object);
    let Some(bytes) = checked(&object.join("shader"), &manifest.digest)? else {
        return Ok(None);
    };
    let reflection = match &manifest.reflection {
        Some(hash) => checked(&object.join("reflection.json"), hash)?,
        None => Some(Vec::new()),
    };
    let Some(reflection) = reflection else {
        return Ok(None);
    };
    if prepared.task.publish_output {
        publish(&prepared.task.destination, &bytes)?;
    }
    if let Some(path) = &prepared.task.reflection {
        publish(path, &reflection)?;
    }
    Ok(Some(Compiled {
        path: object.join("shader"),
        fingerprint: manifest.object.clone(),
        cache_hit: true,
    }))
}

fn object_identity(key: &str, manifest: &Manifest) -> String {
    let mut hash = blake3::Hasher::new();
    field(&mut hash, key.as_bytes());
    field(&mut hash, manifest.version.as_bytes());
    for dep in &manifest.dependencies {
        field(&mut hash, dep.path.as_bytes());
        field(&mut hash, dep.digest.as_bytes());
    }
    for extension in &manifest.extensions {
        field(&mut hash, extension.as_bytes());
    }
    field(&mut hash, manifest.inventory.as_bytes());
    field(&mut hash, manifest.digest.as_bytes());
    if let Some(reflection) = &manifest.reflection {
        field(&mut hash, reflection.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}

fn load(path: &Path) -> Result<Option<Manifest>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(failure(path, e)),
    }
}
fn checked(path: &Path, hash: &str) -> Result<Option<Vec<u8>>, Error> {
    match fs::read(path) {
        Ok(bytes) => Ok((blake3::hash(&bytes).to_hex().as_str() == hash).then_some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(failure(path, e)),
    }
}
fn publish(path: &Path, bytes: &[u8]) -> Result<(), Error> {
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
struct Temporary(PathBuf);
impl Temporary {
    fn new(parent: &Path) -> Result<Self, Error> {
        loop {
            let path = parent.join(format!(
                "{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(failure(&path, e)),
            }
        }
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
