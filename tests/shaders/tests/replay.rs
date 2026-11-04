//! 原始失败 bits 的生产 shader 重放协议回归。
use shader_tests::{input, run};
use simple_engine::gpu_test::{self, DEFAULT_SEED, Failure};

#[test]
#[ignore = "需要 Vulkan compute 设备"]
fn replay_uses_recorded_input_bits() {
    let mut input = input::surfaces(1, DEFAULT_SEED, 2);
    // 故意反转对照法线，构造可复现的性质失败。
    for word in &mut input[20..23] {
        *word ^= 1 << 31;
    }
    let batch = run::compute(input, 48, "surface_main").unwrap();
    gpu_test::shutdown_shared().unwrap();
    assert_ne!(batch.actual[0][0] & 2, 0);
    let record = Failure {
        case: "surface_normal".into(),
        schema: "surface-v1".into(),
        // 记录不依赖 seed 再生成；这里特意与原输入不同。
        seed: [255; 32],
        stream: 99,
        index: 12345,
        input: batch.input,
        actual: batch.actual[0].to_vec(),
        device: batch.device,
        driver: batch.driver,
        shader_blake3: batch.shader_blake3,
    };
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/shader-tests/replay-contract");
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join(format!("{}-input.json", std::process::id()));
    let output = directory.join(format!("{}-output.json", std::process::id()));
    record.save(&source).unwrap();
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_shader_bench"))
        .arg("--replay")
        .arg(&source)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(
        status.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert_eq!(result["passed"], false);
    assert_eq!(result["actual"][0], serde_json::json!(record.actual));
}
