//! 真实 CPU packet 与生产 Slang 的离散光源分布回归；GPU 用例由验收入口串行启用。
use simple_engine::{
    gpu_test::{Capability, Gpu, Shader},
    light_bvh::{Quad, Tree},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

const SELECTORS: u32 = 1 << 24;
const STEP: f32 = 1.0 / SELECTORS as f32;

struct Query {
    tree: usize,
    point: [f32; 3],
    sample: f32,
    reverse: u32,
}

fn quad(x: f32, power: f32) -> Quad {
    Quad {
        vertices: [[x, 0.0, 0.0], [x, 1.0, 0.0], [x, 1.0, 1.0], [x, 0.0, 1.0]],
        power,
    }
}

fn run(trees: &[Tree], queries: &[Query]) -> Result<Vec<[u32; 4]>> {
    let mut words = vec![0; 4 + queries.len() * 8];
    words[0] = queries.len() as u32;
    let bases: Vec<_> = trees
        .iter()
        .map(|tree| {
            words.resize(words.len().next_multiple_of(8), 0);
            let base = words.len() as u32 / 4;
            words.extend_from_slice(tree.words());
            base
        })
        .collect();
    for (index, query) in queries.iter().enumerate() {
        let at = 4 + index * 8;
        words[at..at + 8].copy_from_slice(&[
            bases[query.tree],
            trees[query.tree].len(),
            query.point[0].to_bits(),
            query.point[1].to_bits(),
            query.point[2].to_bits(),
            query.sample.to_bits(),
            query.reverse,
            0,
        ]);
    }
    let input: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
    let shader = Shader {
        spirv: include_bytes!(concat!(env!("OUT_DIR"), "/light_bvh_main.spv")),
        entry: "light_bvh_main",
        workgroup: [64, 1, 1],
    };
    let mut gpu = Gpu::new(Capability::Compute, 0, false)?;
    let output = gpu.compute(
        shader,
        &input,
        queries.len() * 16,
        [(queries.len() as u32).div_ceil(64), 1, 1],
    )?;
    gpu.finish()?;
    assert_eq!(output.len(), queries.len() * 16);
    let rows: Vec<_> = output
        .as_chunks::<16>()
        .0
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let row: [u32; 4] = std::array::from_fn(|channel| {
                u32::from_le_bytes(row[channel * 4..channel * 4 + 4].try_into().unwrap())
            });
            assert_eq!(row[3], index as u32 + 1, "GPU 执行标识缺失");
            row
        })
        .collect();
    Ok(rows)
}

#[test]
#[ignore = "需要 Vulkan compute；由 GPU 验收串行运行"]
fn empty_and_single_light_have_exact_discrete_probabilities() -> Result {
    let trees = [Tree::build(&[])?, Tree::build(&[quad(1.0, 2.0)])?];
    let queries: Vec<_> = [0, 1]
        .into_iter()
        .flat_map(|tree| {
            [0.0, 1.0 - STEP].map(|sample| Query {
                tree,
                point: [0.0; 3],
                sample,
                reverse: if tree == 0 { u32::MAX } else { 0 },
            })
        })
        .collect();
    let rows = run(&trees, &queries)?;
    for (row, query) in rows.iter().zip(&queries) {
        let expected = if query.tree == 0 {
            [u32::MAX, 0, 0]
        } else {
            [0, 1.0_f32.to_bits(), 1.0_f32.to_bits()]
        };
        assert_eq!(row[..3], expected);
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；由 GPU 验收串行运行"]
fn receiver_distance_changes_selection_and_reverse_pmf() -> Result {
    let tree = Tree::build(&[quad(1.0, 1.0), quad(3.0, 1.0)])?;
    const SAMPLES: u32 = 1024;
    let queries: Vec<_> = [0.0, 4.0]
        .into_iter()
        .flat_map(|x| {
            (0..SAMPLES).map(move |index| Query {
                tree: 0,
                point: [x, 0.5, 0.5],
                sample: (index as f32 + 0.5) / SAMPLES as f32,
                reverse: index & 1,
            })
        })
        .collect();
    let rows = run(&[tree], &queries)?;
    // 两灯等功率，距接收点分别为 1 和 3，解析离散权重为 9:1；另一侧交换此比例。
    let near_count = (f64::from(SELECTORS) * 0.9).round_ties_even() as u32;
    let near = near_count as f32 * STEP;
    for (receiver, batch) in rows.chunks(SAMPLES as usize).enumerate() {
        let pmf = if receiver == 0 {
            [near, 1.0 - near]
        } else {
            [1.0 - near, near]
        };
        let mut hits = [0_u32; 2];
        for (index, row) in batch.iter().enumerate() {
            let selected = row[0] as usize;
            assert!(selected < 2, "选灯索引越界：{row:?}");
            hits[selected] += 1;
            assert_eq!(row[1], pmf[selected].to_bits(), "前向 PMF 未匹配解析分布");
            assert_eq!(
                row[2],
                pmf[index & 1].to_bits(),
                "反向 PMF 未依赖同一接收点"
            );
        }
        assert!((hits[0] as f32 / SAMPLES as f32 - pmf[0]).abs() <= 1.0 / SAMPLES as f32);
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；由 GPU 验收串行运行"]
fn half_selector_ties_round_to_even_in_selection_and_replay() -> Result {
    // 等距离时，这两个功率分别产生 100.5 / 2^24、101.5 / 2^24 的首灯概率。
    let trees = [0x36c9_004f, 0x36cb_0051]
        .map(|bits| Tree::build(&[quad(1.0, f32::from_bits(bits)), quad(1.0, 1.0)]))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let counts = [100_u32, 102];
    let queries: Vec<_> = counts
        .into_iter()
        .enumerate()
        .flat_map(|(tree, count)| {
            [(count - 1, 0), (count, 1)].map(|(selector, reverse)| Query {
                tree,
                point: [0.0, 0.5, 0.5],
                sample: selector as f32 * STEP,
                reverse,
            })
        })
        .collect();
    let rows = run(&trees, &queries)?;
    for (row, query) in rows.iter().zip(&queries) {
        let first = counts[query.tree] as f32 * STEP;
        let expected = if query.reverse == 0 {
            first
        } else {
            1.0 - first
        };
        assert_eq!(row[0], query.reverse, "ties-even 区间端点错误");
        assert_eq!(row[1], expected.to_bits());
        assert_eq!(row[2], expected.to_bits());
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；由 GPU 验收串行运行"]
fn receiver_at_leaf_centroid_keeps_power_distribution_finite() -> Result {
    let tree = Tree::build(&[quad(1.0, 1.0), quad(1.0, 3.0)])?;
    // 接收点恰为两个重合正方形的面积质心，距离评分须使用非零备用尺度。
    let center = 0.5;
    let queries: Vec<_> = [(0.0, 0), (0.5, 1)]
        .map(|(sample, reverse)| Query {
            tree: 0,
            point: [1.0, center, center],
            sample,
            reverse,
        })
        .into();
    let rows = run(&[tree], &queries)?;
    for (row, query) in rows.iter().zip(&queries) {
        let expected = if query.reverse == 0 {
            0.25_f32
        } else {
            0.75_f32
        };
        assert_eq!(row[0], query.reverse);
        assert_eq!(row[1], expected.to_bits());
        assert_eq!(row[2], expected.to_bits());
    }
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；由 GPU 验收串行运行"]
fn rare_lights_retain_selector_support_and_matching_reverse_pmf() -> Result {
    let tree = Tree::build(&[
        quad(1.0, 1.0e-30),
        quad(3.0, 1.0e-20),
        quad(5.0, 1.0),
        quad(7.0, 1.0e30),
    ])?;
    let queries: Vec<_> = (0..4)
        .map(|light| Query {
            tree: 0,
            point: [0.0, 0.5, 0.5],
            sample: 0.5,
            reverse: light,
        })
        .chain((0..64).map(|selector| Query {
            tree: 0,
            point: [0.0, 0.5, 0.5],
            sample: selector as f32 * STEP,
            reverse: u32::MAX,
        }))
        .collect();
    let rows = run(&[tree], &queries)?;
    let pmf: [f32; 4] = std::array::from_fn(|index| f32::from_bits(rows[index][2]));
    assert!(pmf.iter().all(|&probability| probability >= STEP));
    assert_eq!(pmf.into_iter().sum::<f32>(), 1.0);
    assert_eq!(pmf.into_iter().reduce(f32::min), Some(STEP));
    let mut seen = [false; 4];
    for row in &rows[4..] {
        let selected = row[0] as usize;
        assert!(selected < 4, "选灯索引越界：{row:?}");
        seen[selected] = true;
        assert_eq!(row[1], pmf[selected].to_bits());
        assert_eq!(row[2], 0, "不在灯目录中的索引 PMF 须为零");
    }
    assert_eq!(seen, [true; 4], "稀有灯的实际格点不得被吞掉");
    Ok(())
}

#[test]
#[ignore = "需要 Vulkan compute；由 GPU 验收串行运行"]
fn median_subtrees_keep_uniform_support_across_partial_blocks() -> Result {
    let counts = [6, 7, 8, 9, 10, 127];
    let trees: Vec<_> = counts
        .map(|count| Tree::build(&vec![quad(1.0, 1.0); count]))
        .into_iter()
        .collect::<std::result::Result<_, _>>()?;
    // 重合等功率灯使所有分支的距离相同；量化只允许少数格点误差，不能丢失奇数侧的叶。
    let queries: Vec<_> = counts
        .into_iter()
        .enumerate()
        .flat_map(|(tree, count)| {
            (0..count).map(move |light| Query {
                tree,
                point: [0.0, 0.5, 0.5],
                sample: (light as f32 + 0.5) / count as f32,
                reverse: light as u32,
            })
        })
        .collect();
    let rows = run(&trees, &queries)?;
    let mut total = vec![0.0; counts.len()];
    for (row, query) in rows.iter().zip(&queries) {
        assert_eq!(row[0], query.reverse);
        assert_eq!(row[1], row[2]);
        let probability = f32::from_bits(row[1]);
        assert!((probability - 1.0 / counts[query.tree] as f32).abs() <= 2.0 * STEP);
        total[query.tree] += probability;
    }
    assert!(total.into_iter().all(|probability| probability == 1.0));
    Ok(())
}
