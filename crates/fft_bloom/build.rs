//! 原生泛光的固定程序，CPU 核生成与读取可独立使用。

#[cfg(not(feature = "vulkan"))]
fn main() {}

#[cfg(feature = "vulkan")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use shader_build::{Recipe, Target};
    use std::{fmt::Write as _, path::PathBuf};
    let mut recipes = Vec::from(
        [
            ("kernel_rows_main", "kernel_rows.spv"),
            ("kernel_columns_main", "kernel_columns.spv"),
            ("forward_rows_main", "forward_rows.spv"),
            ("filter_columns_main", "filter_columns.spv"),
            ("composite_rows_main", "composite_rows.spv"),
            ("wide_kernel_rows_main", "wide_kernel_rows.spv"),
            ("wide_kernel_columns_main", "wide_kernel_columns.spv"),
            ("wide_forward_rows_main", "wide_forward_rows.spv"),
            ("wide_filter_columns_main", "wide_filter_columns.spv"),
            ("wide_composite_rows_main", "wide_composite_rows.spv"),
        ]
        .map(|(entry, output)| Recipe {
            source: "shaders/bloom_compute.slang",
            entry,
            output,
            stage: "compute",
            target: Target::Spirv("spirv_1_4"),
            args: &["-fp-mode", "precise"],
        }),
    );
    // 行与列分别打包。构建表只列实际着色器，宿主直接从同一张表选长度。
    let rows = [
        (256, 128),
        (1024, 128),
        (1920, 192),
        (2304, 128),
        (2880, 192),
        (3584, 256),
        (3840, 128),
        (4800, 192),
        (5760, 192),
        (6144, 256),
    ];
    let columns = [
        (256, 128),
        (1024, 128),
        (1152, 128),
        (1440, 96),
        (1920, 192),
        (2048, 256),
        (2400, 160),
        (2560, 128),
        (3072, 256),
        (3840, 128),
        (5376, 256),
    ];
    let mut table = String::from("const RECT_PROGRAMS: RectPrograms<'static> = RectPrograms {");
    let mut variants = Vec::new();
    for (name, axes, entries) in [
        (
            "rows",
            rows.as_slice(),
            ["rect_kernel_rows", "rect_forward_rows", "rect_inverse_rows"].as_slice(),
        ),
        (
            "columns",
            columns.as_slice(),
            ["rect_kernel_columns", "rect_filter_columns"].as_slice(),
        ),
    ] {
        write!(table, "{name}: &[")?;
        for &(length, lanes) in axes {
            let lanes: u32 = lanes;
            let lane_radix = lanes >> lanes.trailing_zeros();
            let mut factors = [1; 4];
            let mut slots = length / lanes;
            for factor in &mut factors {
                if let Some(divisor) = [2, 3, 5, 7].into_iter().find(|d| slots.is_multiple_of(*d)) {
                    *factor = divisor;
                    slots /= divisor;
                }
            }
            assert!(
                length.is_multiple_of(lanes)
                    && slots == 1
                    && lanes <= 256
                    && lanes.is_multiple_of(4)
                    && matches!(lane_radix, 1 | 3 | 5)
            );
            let mut args = vec![
                "-fp-mode".to_owned(),
                "precise".to_owned(),
                format!("-DFFT_LANES={lanes}"),
                format!("-DFFT_LANE_RADIX={lane_radix}"),
            ];
            args.extend(
                factors
                    .iter()
                    .enumerate()
                    .map(|(i, r)| format!("-DFFT_R{i}={r}")),
            );
            write!(table, "({length}, {lanes}, [")?;
            for &entry in entries {
                let output = format!("{entry}_{length}.spv");
                write!(
                    table,
                    "RayShader {{ spirv: include_bytes!(concat!(env!(\"OUT_DIR\"),\"/{output}\")), entry: c\"{entry}\" }},"
                )?;
                variants.push((entry, output, args.clone()));
            }
            table.push_str("]),");
        }
        table.push_str("],");
    }
    table.push_str("};");
    let mut pupil_table = String::from("const FFT: &[(u32,[RayShader<'static>;2])] = &[");
    if cfg!(feature = "calibration_gpu") {
        for entry in [
            "interpolate_rows",
            "integrate_cells",
            "project_pixels",
            "square_pixels",
            "rasterize_pupil",
        ] {
            variants.push((
                entry,
                format!("{entry}.spv"),
                vec!["-fp-mode".into(), "precise".into()],
            ));
        }
        for n in [128, 256, 512, 1024, 2048, 4096, 8192] {
            write!(pupil_table, "({n},[")?;
            for entry in ["pupil_rows", "pupil_columns"] {
                let output = format!("{entry}_{n}.spv");
                write!(
                    pupil_table,
                    "RayShader{{spirv:include_bytes!(concat!(env!(\"OUT_DIR\"),\"/{output}\")),entry:c\"{entry}\"}},"
                )?;
                variants.push((
                    entry,
                    output,
                    vec![
                        format!("-DPUPIL_FFT_SIZE={n}"),
                        "-fp-mode".into(),
                        "precise".into(),
                    ],
                ));
            }
            pupil_table.push_str("]),");
        }
    }
    pupil_table.push_str("];");
    let args: Vec<Vec<_>> = variants
        .iter()
        .map(|(_, _, args)| args.iter().map(String::as_str).collect())
        .collect();
    recipes.extend(
        variants
            .iter()
            .zip(&args)
            .map(|((entry, output, _), args)| Recipe {
                source: match *entry {
                    "rasterize_pupil" | "pupil_rows" | "pupil_columns" => {
                        "shaders/pupil_fourier.slang"
                    }
                    "interpolate_rows" | "integrate_cells" | "project_pixels" | "square_pixels" => {
                        "shaders/pupil_integral.slang"
                    }
                    _ => "shaders/rect_compute.slang",
                },
                entry,
                output,
                stage: "compute",
                target: Target::Spirv("spirv_1_4"),
                args,
            }),
    );
    shader_build::build(
        &recipes,
        &["shaders", "../../shaders"],
        &["src", "data/camara.rgb32le", "data/pupil.rgb32le"],
    )?;
    let out =
        PathBuf::from(std::env::var_os("OUT_DIR").ok_or(shader_build::Error::OutputDirectory)?);
    std::fs::write(out.join("rect_programs.rs"), table)?;
    std::fs::write(out.join("pupil_programs.rs"), pupil_table)?;
    Ok(())
}
