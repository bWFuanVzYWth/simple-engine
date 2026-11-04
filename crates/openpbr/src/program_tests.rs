use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static SERIAL: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/openpbr-program-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("main.slang"),
            "#ifndef OPENPBR_VARIANT\n#define OPENPBR_VARIANT VARIANT\n#endif\nstruct Payload { uint value; }; [shader(\"callable\")] void shade(inout Payload data) { data.value = OPENPBR_FLOW + OPENPBR_VARIANT; }").unwrap();
        Self(root)
    }

    fn compile(&self, compiler: &RuntimeCompiler, args: &[&str]) -> ProgramSet {
        let flow = crate::Material::opaque([0.5; 3], 0.9, 1.5, 0.0, 0.0)
            .unwrap()
            .flow(false);
        ProgramSet::compile(
            compiler,
            [flow, flow],
            Template {
                source: self.0.join("main.slang").to_str().unwrap(),
                entry: "shade",
                output: "material.spv",
                target: Target::Spirv("spirv_1_4"),
                args,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "需要已安装的 Slang；验证共同程序集的实际变体和缓存身份"]
fn material_program_variants_have_distinct_code_identities() {
    let fixture = Fixture::new();
    let compiler = RuntimeCompiler::new(
        fixture.0.join("cache"),
        &[],
        &[&fixture.0.join("main.slang")],
    )
    .unwrap();
    let first = fixture.compile(&compiler, &["-DVARIANT=0", "-fp-mode", "precise"]);
    let same = fixture.compile(&compiler, &["-DVARIANT=0", "-fp-mode", "precise"]);
    let variant = fixture.compile(&compiler, &["-DVARIANT=1", "-fp-mode", "precise"]);
    assert!(same.same_code(&first));
    assert!(same.artifacts()[0].cache_hit());
    assert!(!variant.same_code(&first));
    assert_ne!(variant.artifacts()[0].bytes(), first.artifacts()[0].bytes());
    assert_eq!(first.plan().bindings(), &[0, 0]);
}

#[test]
#[ignore = "需要已安装的 Slang；同一 Flow 的不同场景能力须分配独立 callable"]
fn adapter_variants_split_a_single_flow() {
    let fixture = Fixture::new();
    let source = fixture.0.join("main.slang");
    let compiler = RuntimeCompiler::new(fixture.0.join("cache"), &[], &[&source]).unwrap();
    let flow = crate::Material::opaque([0.5; 3], 0.9, 1.5, 0.0, 0.0)
        .unwrap()
        .flow(false);
    let programs = ProgramSet::compile_variants(
        &compiler,
        [(flow, 4), (flow, 0), (flow, 4)],
        Template {
            source: source.to_str().unwrap(),
            entry: "shade",
            output: "material.spv",
            target: Target::Spirv("spirv_1_4"),
            args: &["-fp-mode", "precise"],
        },
    )
    .unwrap();
    assert_eq!(programs.plan().bindings(), &[1, 0, 1]);
    let [plain, adapter] = programs.artifacts() else {
        panic!("相同的材质能力应共享一个 callable");
    };
    assert_ne!(plain.bytes(), adapter.bytes());
}
