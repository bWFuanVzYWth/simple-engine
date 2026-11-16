use super::*;
use std::sync::OnceLock;

#[test]
fn task_rejects_an_output_without_a_filename() {
    let result = Task::new("main", vec!["main.slang".into()], PathBuf::from(".."));
    assert!(matches!(result, Err(Error::Configuration(_))));
}

#[test]
fn task_rejects_missing_source() {
    let result = Task::new("main", vec![], PathBuf::from("main.spv"));
    assert!(matches!(result, Err(Error::Configuration(_))));
}

struct Fixture {
    root: PathBuf,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let parent = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/shader-batch-tests");
        fs::create_dir_all(&parent).unwrap();
        let root = TemporaryDirectory::new(&parent).unwrap();
        let root_path = root.path.clone();
        std::mem::forget(root);
        let source = root_path.join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("main.slang"), "module main; import constant; var output: RWStructuredBuffer<uint>; [shader(\"compute\")] [numthreads(1, 1, 1)] func main() -> void { output[0] = value() + VALUE; }").unwrap();
        fs::write(
            source.join("constant.slang"),
            "module constant; public func value() -> uint { return 17; }",
        )
        .unwrap();
        Self {
            root: root_path,
            source,
        }
    }
    fn output(&self, name: &str) -> PathBuf {
        let path = self.root.join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn task(&self, output: &Path, value: u32, reflection: bool) -> Task {
        let mut args = vec![
            command_path(&self.source.join("main.slang")),
            "-std".into(),
            "2026".into(),
            "-target".into(),
            "spirv".into(),
            "-profile".into(),
            "spirv_1_5".into(),
            "-emit-spirv-directly".into(),
            "-entry".into(),
            "main".into(),
            "-stage".into(),
            "compute".into(),
            "-g0".into(),
            "-I".into(),
            command_path(&self.source),
            "-I".into(),
            command_path(output),
            format!("-DVALUE={value}"),
        ];
        if reflection {
            args.extend([
                "-reflection-json".into(),
                command_path(&output.join(format!("{value}.json"))),
            ]);
        }
        Task::new("main", args, output.join(format!("{value}.spv"))).unwrap()
    }
    fn paired_task(&self, output: &Path, value: u32) -> Task {
        let task = self.task(output, value, true);
        let mut args = task.arguments;
        for arg in &mut args {
            if arg == "-g0" {
                *arg = "-g3".into();
            }
        }
        args.push("-separate-debug-info".into());
        Task::new("main", args, task.destination).unwrap()
    }
    fn run(&self, output: &Path, tasks: &[Task], jobs: usize) -> String {
        static COMPILER: OnceLock<Compiler> = OnceLock::new();
        let compiler = COMPILER.get_or_init(|| Compiler::find(false).unwrap());
        let scheduler = Scheduler {
            jobs,
            server: Some(jobserver::Client::new(jobs.saturating_sub(1)).unwrap()),
        };
        compile_in(
            compiler,
            &scheduler,
            output,
            tasks,
            &self.root.join("cache"),
        )
        .unwrap()
    }
}

#[test]
fn publication_prepares_the_whole_group_before_replacing_outputs() {
    let fixture = Fixture::new();
    let main = fixture.root.join("main");
    let invalid = fixture.root.join("directory");
    fs::write(&main, b"old").unwrap();
    fs::create_dir(&invalid).unwrap();
    assert!(publish_files(&[(&main, b"new"), (&invalid, b"audit")]).is_err());
    assert_eq!(fs::read(main).unwrap(), b"old");
}

#[test]
#[ignore = "需要Slang；不创建GPU"]
fn paired_cache_restores_both_outputs_in_a_new_slot() {
    let fixture = Fixture::new();
    let first = fixture.output("first");
    let second = fixture.output("second");
    assert!(
        fixture
            .run(&first, &[fixture.paired_task(&first, 1)], 1)
            .contains("compiled=1")
    );
    assert!(
        fixture
            .run(&second, &[fixture.paired_task(&second, 1)], 1)
            .contains("cache_hits=1")
    );
    for file in ["1.spv", "1.audit.spv", "1.json"] {
        assert_eq!(
            fs::read(first.join(file)).unwrap(),
            fs::read(second.join(file)).unwrap()
        );
    }
    super::super::verify_audit_pair(
        &fs::read(second.join("1.spv")).unwrap(),
        &fs::read(second.join("1.audit.spv")).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "需要Slang；不创建GPU"]
fn missing_or_corrupt_cached_sidecar_recompiles_the_pair() {
    let fixture = Fixture::new();
    let output = fixture.output("out");
    fixture.run(&output, &[fixture.paired_task(&output, 1)], 1);
    for missing in [true, false] {
        for object in fs::read_dir(fixture.root.join("cache/objects")).unwrap() {
            let audit = object.unwrap().path().join("audit.spv");
            if missing {
                fs::remove_file(audit).unwrap();
            } else {
                fs::write(audit, b"corrupt").unwrap();
            }
        }
        assert!(
            fixture
                .run(&output, &[fixture.paired_task(&output, 1)], 1)
                .contains("compiled=1")
        );
        super::super::verify_audit_pair(
            &fs::read(output.join("1.spv")).unwrap(),
            &fs::read(output.join("1.audit.spv")).unwrap(),
        )
        .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn make_dependencies_accept_windows_paths_spaces_and_continuations() {
    let paths = dependencies(
        b"C:/out/test.spv: C:/src/main.slang C:/a\\ b/header.h \\\n C:\\src\\other.slang\n",
    )
    .unwrap();
    assert_eq!(
        paths,
        [
            PathBuf::from("C:/src/main.slang"),
            PathBuf::from("C:/a b/header.h"),
            PathBuf::from(r"C:\src\other.slang")
        ]
    );
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn cached_reflection_survives_a_different_output_slot() {
    let f = Fixture::new();
    let a = f.output("first");
    let b = f.output("second");
    assert!(f.run(&a, &[f.task(&a, 1, true)], 1).contains("compiled=1"));
    assert!(
        f.run(&b, &[f.task(&b, 1, true)], 1)
            .contains("cache_hits=1")
    );
    assert_eq!(
        read_file(&a.join("1.spv")).unwrap(),
        read_file(&b.join("1.spv")).unwrap()
    );
    assert_eq!(
        read_file(&a.join("1.json")).unwrap(),
        read_file(&b.join("1.json")).unwrap()
    );
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn only_consumers_of_changed_modules_are_recompiled() {
    let f = Fixture::new();
    let out = f.output("out");
    fs::write(f.source.join("other.slang"), "module other; var output: RWStructuredBuffer<uint>; [shader(\"compute\")] [numthreads(1, 1, 1)] func main() -> void { output[0] = 9; }").unwrap();
    let one = f.task(&out, 1, false);
    let mut two = f.task(&out, 2, false);
    two.arguments[0] = command_path(&f.source.join("other.slang"));
    let tasks = [one, two];
    assert!(f.run(&out, &tasks, 2).contains("compiled=2"));
    fs::write(
        f.source.join("constant.slang"),
        "module constant; public func value() -> uint { return 18; }",
    )
    .unwrap();
    let result = f.run(&out, &tasks, 2);
    assert!(
        result.contains("cache_hits=1") && result.contains("compiled=1"),
        "{result}"
    );
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn new_generated_include_shadows_the_cached_module() {
    let f = Fixture::new();
    let out = f.output("out");
    let isolated = f.output("main-only");
    fs::copy(f.source.join("main.slang"), isolated.join("main.slang")).unwrap();
    let task = || {
        let mut task = f.task(&out, 1, false);
        task.arguments[0] = command_path(&isolated.join("main.slang"));
        let n = task.arguments.len();
        task.arguments.swap(n - 4, n - 2);
        task
    };
    f.run(&out, &[task()], 1);
    let first = read_file(&out.join("1.spv")).unwrap();
    fs::write(
        out.join("constant.slang"),
        "module constant; public func value() -> uint { return 23; }",
    )
    .unwrap();
    assert!(f.run(&out, &[task()], 1).contains("compiled=1"));
    assert_ne!(first, read_file(&out.join("1.spv")).unwrap());
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn parallel_compilation_matches_serial_outputs_and_reuses_them() {
    let f = Fixture::new();
    let out = f.output("out");
    let tasks: Vec<_> = (0..8).map(|v| f.task(&out, v, false)).collect();
    let cold = f.run(&out, &tasks, 4);
    assert!(cold.contains("compiled=8"), "{cold}");
    assert!(!cold.contains("peak_tasks=1\n"), "{cold}");
    let warm = f.run(&out, &tasks, 1);
    assert!(warm.contains("cache_hits=8"), "{warm}");
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn corrupted_cache_is_repaired_instead_of_published() {
    let f = Fixture::new();
    let out = f.output("out");
    f.run(&out, &[f.task(&out, 1, false)], 1);
    let before = read_file(&out.join("1.spv")).unwrap();
    for object in fs::read_dir(f.root.join("cache/objects")).unwrap() {
        fs::write(object.unwrap().path().join("shader"), b"corrupt").unwrap();
    }
    assert!(
        f.run(&out, &[f.task(&out, 1, false)], 1)
            .contains("compiled=1")
    );
    assert_eq!(before, read_file(&out.join("1.spv")).unwrap());
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn generated_module_content_disambiguates_output_slots() {
    let f = Fixture::new();
    fs::write(f.source.join("main.slang"), "module main; import generated; var output: RWStructuredBuffer<uint>; [shader(\"compute\")] [numthreads(1, 1, 1)] func main() -> void { output[0] = generated_value(); }").unwrap();
    let a = f.output("first");
    let b = f.output("second");
    let same = "module generated; public func generated_value() -> uint { return 11; }";
    fs::write(a.join("generated.slang"), same).unwrap();
    fs::write(b.join("generated.slang"), same).unwrap();
    assert!(f.run(&a, &[f.task(&a, 1, false)], 1).contains("compiled=1"));
    assert!(
        f.run(&b, &[f.task(&b, 1, false)], 1)
            .contains("cache_hits=1")
    );
    fs::write(
        b.join("generated.slang"),
        "module generated; public func generated_value() -> uint { return 12; }",
    )
    .unwrap();
    assert!(f.run(&b, &[f.task(&b, 1, false)], 1).contains("compiled=1"));
    assert_ne!(
        read_file(&a.join("1.spv")).unwrap(),
        read_file(&b.join("1.spv")).unwrap()
    );
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn failed_compilation_does_not_publish_a_cache_entry() {
    let f = Fixture::new();
    let out = f.output("out");
    fs::write(
        f.source.join("constant.slang"),
        "module constant; public invalid syntax",
    )
    .unwrap();
    let compiler = Compiler::find(false).unwrap();
    let result = compile_in(
        &compiler,
        &Scheduler {
            jobs: 1,
            server: None,
        },
        &out,
        &[f.task(&out, 1, false)],
        &f.root.join("cache"),
    );
    assert!(matches!(result, Err(Error::Compile { .. })));
    assert!(!out.join("1.spv").exists());
    assert_eq!(
        fs::read_dir(f.root.join("cache/objects")).unwrap().count(),
        0
    );
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn measure_cold_serial_parallel_and_warm_batches() {
    let serial = Fixture::new();
    let parallel = Fixture::new();
    let a = serial.output("out");
    let b = parallel.output("out");
    let tasks_a: Vec<_> = (0..16).map(|v| serial.task(&a, v, false)).collect();
    let tasks_b: Vec<_> = (0..16).map(|v| parallel.task(&b, v, false)).collect();
    let one = serial.run(&a, &tasks_a, 1);
    let many = parallel.run(&b, &tasks_b, 4);
    let warm = parallel.run(&b, &tasks_b, 4);
    for value in 0..16 {
        assert_eq!(
            read_file(&a.join(format!("{value}.spv"))).unwrap(),
            read_file(&b.join(format!("{value}.spv"))).unwrap()
        );
    }
    println!("serial\n{one}parallel\n{many}warm\n{warm}");
}

#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn restoring_a_previous_source_version_reuses_its_artifact() {
    let f = Fixture::new();
    let out = f.output("out");
    let path = f.source.join("constant.slang");
    let original = read_file(&path).unwrap();
    f.run(&out, &[f.task(&out, 1, true)], 1);
    let first = read_file(&out.join("1.spv")).unwrap();
    fs::write(
        &path,
        "module constant; public func value() -> uint { return 18; }",
    )
    .unwrap();
    assert!(
        f.run(&out, &[f.task(&out, 1, true)], 1)
            .contains("compiled=1")
    );
    fs::write(&path, original).unwrap();
    assert!(
        f.run(&out, &[f.task(&out, 1, true)], 1)
            .contains("cache_hits=1")
    );
    assert_eq!(first, read_file(&out.join("1.spv")).unwrap());
}
#[test]
#[ignore = "需要 Slang；不创建 GPU"]
fn concurrent_output_slots_publish_one_shared_cache_entry() {
    let f = Fixture::new();
    let a = f.output("first");
    let b = f.output("second");
    let (one, two) = thread::scope(|scope| {
        let first = scope.spawn(|| f.run(&a, &[f.task(&a, 1, true)], 1));
        let second = f.run(&b, &[f.task(&b, 1, true)], 1);
        (first.join().unwrap(), second)
    });
    let hits =
        usize::from(one.contains("cache_hits=1")) + usize::from(two.contains("cache_hits=1"));
    assert_eq!(hits, 1, "{one}\n{two}");
    assert_eq!(
        read_file(&a.join("1.spv")).unwrap(),
        read_file(&b.join("1.spv")).unwrap()
    );
    assert_eq!(
        read_file(&a.join("1.json")).unwrap(),
        read_file(&b.join("1.json")).unwrap()
    );
}
