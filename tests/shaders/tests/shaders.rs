//! 生产 Slang 数值模块的单元与批量属性检验。
//!
//! 采样、非均匀变换表面各生成一批输入，默认每批 65,536 个；同一批的独立性质共享
//! 一次执行结果，不重复提交 GPU。初始化错误也缓存，环境生命周期交给测试设施。
//! `GPU_TEST_CASES` 只调整这两批；三个 RT 场景各固定 4,096 条，分别关注双侧出射、
//! 长距离掠射连接及近端遮挡。所有用例保留 ignored/filter 语义，运行结束显式清理环境。
//!
//! 失败记录保存实际输入而非只记 seed：统计性质保留整批输入，逐样本性质只保留该输入，
//! RT 另带 AS 几何。CPU 输入生成含三角函数，不保证跨平台逐位一致，重放不得重新生成。
//! 位模式与随机协议见 [`Failure`] 和 [`gpu_test::input`]；输入布局见 [`shader_tests::input`]，
//! 执行标识、性质 mask 与重放范围见 [`shader_tests::run`]。
use shader_tests::{
    input,
    run::{self, Batch},
};
use simple_engine::gpu_test::{self, DEFAULT_SEED, Failure, PropertyRng, unit_f32};
use std::{path::PathBuf, sync::OnceLock};

fn seed() -> [u8; 32] {
    std::env::var("GPU_TEST_SEED").map_or(DEFAULT_SEED, |value| {
        gpu_test::input::parse_seed(&value).expect("GPU_TEST_SEED 无效")
    })
}

fn count() -> u32 {
    std::env::var("GPU_TEST_CASES").map_or(65_536, |value| {
        value.parse().expect("GPU_TEST_CASES 须为正整数")
    })
}

/// 每类批量只初始化一次，性质断言在共享结果上独立进行；错误不触发隐式重试。
fn cached(
    cache: &'static OnceLock<Result<Batch, String>>,
    create: impl FnOnce() -> Result<Batch, run::Error>,
) -> &'static Batch {
    cache
        .get_or_init(|| create().map_err(|error| error.to_string()))
        .as_ref()
        .unwrap_or_else(|error| panic!("{error}"))
}

fn sampling() -> &'static Batch {
    static RESULT: OnceLock<Result<Batch, String>> = OnceLock::new();
    cached(&RESULT, || {
        run::compute(input::sampling(count(), seed(), 1), 16, "properties_main")
    })
}

fn surfaces() -> &'static Batch {
    static RESULT: OnceLock<Result<Batch, String>> = OnceLock::new();
    cached(&RESULT, || {
        run::compute(input::surfaces(count(), seed(), 2), 48, "surface_main")
    })
}

fn known() -> &'static Batch {
    static RESULT: OnceLock<Result<Batch, String>> = OnceLock::new();
    cached(&RESULT, || run::compute(input::known(), 16, "known_main"))
}

/// 保存原始输入、触发样本的结果及 shader 身份后失败；whole 保留整批输入，否则只留该样本。
/// RT 的共享几何前缀随单样本一起保留，记录名加入输入 hash 与时间，避免覆盖旧现场。
fn failure(batch: &Batch, index: usize, case: &str, schema: &str, stream: u64, whole: bool) -> ! {
    let input = if whole {
        batch.input.clone()
    } else {
        batch
            .prefix
            .iter()
            .chain(&batch.input[index * batch.stride..(index + 1) * batch.stride])
            .copied()
            .collect()
    };
    let failure = Failure {
        case: case.into(),
        schema: schema.into(),
        seed: seed(),
        stream,
        index: index as u64,
        input,
        actual: batch.actual[index].to_vec(),
        device: batch.device.clone(),
        driver: batch.driver.clone(),
        shader_blake3: batch.shader_blake3.clone(),
    };
    let directory =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../temp/shader-tests/failures");
    std::fs::create_dir_all(&directory).expect("无法创建失败记录目录");
    let input_hash = blake3::hash(&run::bytes(&failure.input)).to_hex();
    let observed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = directory.join(format!(
        "{case}-{index}-{}-{observed}.json",
        &input_hash[..12]
    ));
    failure.save(&path).expect("无法保存失败记录");
    panic!(
        "{case}：样本 {index}，实际 {:?}，重放记录 {}；Slang {}，源 {}",
        batch.actual[index],
        path.display(),
        shader_tests::SLANG_VERSION,
        shader_tests::SOURCE_BLAKE3,
    );
}

fn flags(batch: &Batch, mask: u32, case: &str, schema: &str, stream: u64) {
    if let Some(index) = batch.actual.iter().position(|row| row[0] & mask != 0) {
        failure(batch, index, case, schema, stream, false);
    }
}

fn random_samples_stay_in_unit_interval() {
    flags(sampling(), 1, "random_unit_interval", "sampling-v1", 1);
}

fn cosine_samples_are_unit_vectors() {
    flags(sampling(), 2, "cosine_unit", "sampling-v1", 1);
}

fn cosine_samples_stay_in_normal_hemisphere() {
    flags(sampling(), 4, "cosine_hemisphere", "sampling-v1", 1);
}

/// 余弦半球的法线投影均值应为 2/3，绝对误差须小于 0.005。
/// 容限按默认大批量设定，不随 GPU_TEST_CASES 缩放；小批量只适合诊断。
fn cosine_samples_have_expected_first_moment() {
    let batch = sampling();
    let mean = batch
        .actual
        .iter()
        .map(|row| f64::from(f32::from_bits(row[2])))
        .sum::<f64>()
        / batch.actual.len() as f64;
    if !mean.is_finite() || (mean - 2.0 / 3.0).abs() >= 0.005 {
        failure(
            batch,
            0,
            "cosine_first_moment",
            "sampling-moment-v1",
            1,
            true,
        );
    }
}

fn lambertian_sampling_preserves_reflectance() {
    flags(sampling(), 8, "lambertian_throughput", "sampling-v1", 1);
}

fn power_mis_weights_are_complementary() {
    flags(sampling(), 16, "mis_complement", "sampling-v1", 1);
}

fn power_mis_matches_one_sample_weights() {
    flags(known(), 1, "mis_known_weights", "known-v1", 0);
}

fn lambertian_eval_matches_diffuse_brdf() {
    flags(known(), 2, "lambertian_known_value", "known-v1", 0);
}

fn lambertian_pdf_matches_solid_angle_density() {
    flags(known(), 4, "lambertian_known_pdf", "known-v1", 0);
}

fn triangle_points_match_affine_barycentric_positions() {
    flags(surfaces(), 1, "surface_position", "surface-v1", 2);
}

fn triangle_normals_match_transformed_geometry() {
    flags(surfaces(), 2, "surface_normal", "surface-v1", 2);
}

fn spawn_origins_straddle_the_reconstructed_surface() {
    flags(surfaces(), 4, "spawn_sides", "surface-v1", 2);
}

fn endpoint_bound_grows_with_ray_coordinates() {
    let batch = run::compute(input::endpoint_bound(), 40, "connection_bound_main").unwrap();
    let [base, near, far, _] = batch.actual[0].map(f32::from_bits);
    if !(near > base && far > near) {
        failure(&batch, 0, "endpoint_bound", "connection-bound-v1", 0, false);
    }
}

fn connection_rejects_degenerate_segments() {
    let batch = run::compute(input::degenerate(), 20, "connection_degenerate_main").unwrap();
    if batch.actual[0][..2] != [0, 0] {
        failure(
            &batch,
            0,
            "connection_degenerate",
            "connection-degenerate-v1",
            0,
            false,
        );
    }
}

fn ray_parameters(tangent: f32, expected: bool, outgoing: bool) -> Vec<u32> {
    let mut rng = PropertyRng::new(seed(), 3);
    let mut parameters = Vec::with_capacity(4096 * 4);
    for index in 0..4096u32 {
        let words = rng.words(u64::from(index));
        let bary = if index < 256 {
            [
                (index as f32 + 0.5) / 1e7,
                ((index * 73 % 256) as f32 + 0.5) / 1e7,
            ]
        } else {
            [
                (unit_f32(words[0]) * 255.0 + 0.5) / 1e7,
                (unit_f32(words[1]) * 255.0 + 0.5) / 1e7,
            ]
        };
        let side = if outgoing {
            if index % 2 == 0 { 1.0 } else { -1.0 }
        } else if expected {
            1.0
        } else {
            0.0
        };
        parameters.extend([bary[0], bary[1], tangent, side].map(f32::to_bits));
    }
    parameters
}

fn surface_outgoing_ray_is_not_self_occluded() {
    let batch = run::rays(
        &[[[0.0, 0.0, 0.0], [2000.0, 0.25, 0.0], [0.0, 0.5, 2000.0]]],
        ray_parameters(0.0, false, true),
        true,
    )
    .unwrap();
    flags(&batch, 3, "outgoing_self_hit", "ray-geometry-v1", 3);
}

fn long_grazing_connection_endpoint_is_not_self_occluded() {
    let batch = run::rays(
        &[[
            [0.0, 0.0, 0.0],
            [2000.0, 1000.0, 0.0],
            [0.0, 1000.0, 2000.0],
        ]],
        ray_parameters(1e5, false, false),
        false,
    )
    .unwrap();
    flags(&batch, 3, "connection_endpoint", "ray-geometry-v1", 3);
}

fn connection_detects_occluder_near_endpoint() {
    let triangle = [[0.0, 0.0, 0.0], [2000.0, 0.25, 0.0], [0.0, 0.5, 2000.0]];
    let occluder = triangle.map(|[x, y, z]| [x, y - 0.01, z]);
    let batch = run::rays(
        &[triangle, occluder],
        ray_parameters(20.0, true, false),
        false,
    )
    .unwrap();
    flags(&batch, 3, "near_occluder", "ray-geometry-v1", 3);
}

fn main() -> std::process::ExitCode {
    macro_rules! cases {
        ($kind:literal: $($name:ident),* $(,)?) => {
            vec![$(gpu_test::Trial::test(stringify!($name), || {
                $name();
                Ok(())
            }).with_kind($kind).with_ignored_flag(true)),*]
        };
    }
    let mut tests = cases!["compute":
        random_samples_stay_in_unit_interval,
        cosine_samples_are_unit_vectors,
        cosine_samples_stay_in_normal_hemisphere,
        cosine_samples_have_expected_first_moment,
        lambertian_sampling_preserves_reflectance,
        power_mis_weights_are_complementary,
        power_mis_matches_one_sample_weights,
        lambertian_eval_matches_diffuse_brdf,
        lambertian_pdf_matches_solid_angle_density,
        triangle_points_match_affine_barycentric_positions,
        triangle_normals_match_transformed_geometry,
        spawn_origins_straddle_the_reconstructed_surface,
        endpoint_bound_grows_with_ray_coordinates,
        connection_rejects_degenerate_segments,
    ];
    tests.extend(cases!["rt":
        surface_outgoing_ray_is_not_self_occluded,
        long_grazing_connection_endpoint_is_not_self_occluded,
        connection_detects_occluder_near_endpoint,
    ]);
    let corpus = std::env::var_os("GPU_TEST_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/regressions"));
    let mut records: Vec<_> = std::fs::read_dir(corpus)
        .expect("无法读取回归输入目录")
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    records.sort();
    for path in records {
        let record = Failure::load(&path).expect("回归输入格式无效");
        let kind = if record.schema == "ray-geometry-v1" {
            "rt"
        } else {
            "compute"
        };
        let name = format!("regression/{}", path.file_stem().unwrap().to_string_lossy());
        tests.push(
            gpu_test::Trial::test(name, move || {
                let result = run::replay(&record).map_err(|error| error.to_string())?;
                if result["passed"] != true {
                    return Err(format!("{result}").into());
                }
                Ok(())
            })
            .with_kind(kind)
            .with_ignored_flag(true),
        );
    }
    gpu_test::run(tests)
}
