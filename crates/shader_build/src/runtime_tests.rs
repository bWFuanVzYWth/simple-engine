use super::*;

struct Fixture {
    root: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/shader-build-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("main.slang");
        fs::write(&source, "RWStructuredBuffer<uint> output; [shader(\"compute\")] [numthreads(1, 1, 1)] void main() { output[0] = VALUE; }").unwrap();
        Self { root, source }
    }

    fn compiler(&self) -> RuntimeCompiler {
        RuntimeCompiler::new(self.root.join("cache"), &[], &[&self.source]).unwrap()
    }

    fn compile(&self, compiler: &RuntimeCompiler) -> Artifact {
        compiler
            .compile(&Recipe {
                source: self.source.to_str().unwrap(),
                entry: "main",
                stage: "compute",
                output: "main.spv",
                target: Target::Spirv("spirv_1_5"),
                args: &["-DVALUE=17", "-g0"],
            })
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn explicit_compiler_fails_at_the_requested_path() {
    let fixture = Fixture::new();
    let executable = fixture.root.join("missing-slangc.exe");
    let result =
        RuntimeCompiler::from_executable(&executable, fixture.root.join("cache"), &[], &[]);
    assert!(matches!(result, Err(Error::Io { path, source })
        if path == executable && source.kind() == io::ErrorKind::NotFound));
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn runtime_reuses_only_identical_compile_inputs() {
    let fixture = Fixture::new();
    let compiler = fixture.compiler();
    let first = fixture.compile(&compiler);
    let second = fixture.compile(&compiler);
    assert!(!first.cache_hit());
    assert!(second.cache_hit());
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.fingerprint(), second.fingerprint());
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn changing_source_creates_a_new_artifact_identity() {
    let fixture = Fixture::new();
    let compiler = fixture.compiler();
    let first = fixture.compile(&compiler);
    fs::write(&fixture.source, "RWStructuredBuffer<uint> output; [shader(\"compute\")] [numthreads(1, 1, 1)] void main() { output[0] = VALUE + 1; }").unwrap();
    let second = fixture.compile(&compiler);
    assert!(!second.cache_hit());
    assert_ne!(first.fingerprint(), second.fingerprint());
    assert_ne!(first.bytes(), second.bytes());
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn corrupt_cached_bytes_are_recompiled() {
    let fixture = Fixture::new();
    let compiler = fixture.compiler();
    let first = fixture.compile(&compiler);
    fs::write(first.path(), "corrupt").unwrap();
    let second = fixture.compile(&compiler);
    assert!(!second.cache_hit());
    assert_eq!(first.bytes(), second.bytes());
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn absolute_include_roots_resolve_imported_modules() {
    let fixture = Fixture::new();
    let include = fixture.root.join("includes");
    fs::create_dir_all(&include).unwrap();
    fs::write(
        include.join("constant.slang"),
        "module constant; public uint value() { return 17; }",
    )
    .unwrap();
    fs::write(&fixture.source, "import constant; RWStructuredBuffer<uint> output; [shader(\"compute\")] [numthreads(1,1,1)] void main() { output[0] = value(); }").unwrap();
    let compiler =
        RuntimeCompiler::new(fixture.root.join("cache"), &[&include], &[&fixture.source]).unwrap();
    let result = fixture.compile(&compiler);
    assert_eq!(&result.bytes()[..4], &0x0723_0203_u32.to_le_bytes());
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn invalid_spirv_is_rejected_before_cache_publication() {
    let fixture = Fixture::new();
    fs::write(&fixture.source, "RWStructuredBuffer<uint> output; [shader(\"compute\")] [numthreads(1,1,1)] void main() { spirv_asm { OpExecutionMode $main OriginUpperLeft; }; output[0] = 1; }").unwrap();
    let compiler = fixture.compiler();
    let result = compiler.compile(&Recipe {
        source: fixture.source.to_str().unwrap(),
        entry: "main",
        stage: "compute",
        output: "main.spv",
        target: Target::Spirv("spirv_1_5"),
        args: &["-g0"],
    });
    let Err(Error::Compile { diagnostic, .. }) = result else {
        panic!("未拒绝无效SPIR-V");
    };
    assert!(diagnostic.contains("Validation of generated SPIR-V failed"));
    assert!(!fs::read_dir(&compiler.cache).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|extension| extension == "spv")
    }));
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn runtime_spirv_debug_policy_matches_the_cargo_profile() {
    let fixture = Fixture::new();
    let compiler = fixture.compiler();
    let recipe = |args| Recipe {
        source: fixture.source.to_str().unwrap(),
        entry: "main",
        stage: "compute",
        output: "main.spv",
        target: Target::Spirv("spirv_1_5"),
        args,
    };
    let default = compiler.compile(&recipe(&["-DVALUE=17"])).unwrap();
    let opposite = if super::super::release_lto() {
        "-g3"
    } else {
        "-g0"
    };
    let other = compiler
        .compile(&recipe(&["-DVALUE=17", opposite]))
        .unwrap();
    let debug = |artifact: &Artifact| {
        artifact
            .bytes()
            .windows(b"NonSemantic.Shader.DebugInfo.100".len())
            .any(|window| window == b"NonSemantic.Shader.DebugInfo.100")
    };
    assert_eq!(debug(&default), !super::super::release_lto());
    assert_ne!(debug(&default), debug(&other));
    assert_ne!(default.fingerprint(), other.fingerprint());
}

#[test]
#[ignore = "需要已安装的 Slang 编译器"]
fn runtime_wgsl_always_disables_debug_retention() {
    let fixture = Fixture::new();
    let compiler = fixture.compiler();
    let recipe = |args| Recipe {
        source: fixture.source.to_str().unwrap(),
        entry: "main",
        stage: "compute",
        output: "main.wgsl",
        target: Target::Wgsl,
        args,
    };
    let plain = compiler.compile(&recipe(&["-DVALUE=17", "-g0"])).unwrap();
    let overridden = compiler.compile(&recipe(&["-DVALUE=17", "-g3"])).unwrap();
    assert_eq!(plain.bytes(), overridden.bytes());
    assert_ne!(plain.fingerprint(), overridden.fingerprint());
}
