//! 真实固定字体的 CPU 排版、栅格、编辑与命中负载，不初始化 GPU。
//!
//! font_new_drop 包含字体复制/解析/析构；其余组排除字体加载及输入准备。
//! layout_miss_eviction 轮换预生成正文，单测布局 miss/淘汰，不宣称字体内部 cache 为冷。
//! raster_miss_eviction 的键数超过固定缓存，保留字体和布局，逐轮重建栅格。
use std::{collections::HashSet, hint::black_box};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use input::{EditUnit, TextMovement, TextOp};
use render2d::{
    Point,
    text::{TextBudgets, TextDocument, TextSelection, TextStyle, TextSystem},
};

const FONT: &[u8] = include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf");
const BODY: &str =
    "体素世界 voxel 0123456789，连续批量计算与独占输出。快速棕色狐狸跳过懒狗；山川草木日月星辰。\n";

fn system(budgets: TextBudgets) -> TextSystem {
    TextSystem::new([FONT.to_vec()], budgets).unwrap()
}

fn layouts(c: &mut Criterion) {
    c.bench_function("text/font_new_drop", |b| {
        b.iter(|| {
            let text = system(TextBudgets::default());
            black_box(text.stats());
        });
    });
    let mut group = c.benchmark_group("text/layout");
    for (name, repeats) in [("label", 1), ("paragraph", 16)] {
        let body = BODY.repeat(repeats);
        let style = TextStyle::default();
        let mut text = system(TextBudgets {
            max_layouts: 8,
            ..Default::default()
        });
        let layout = text.layout(&body, 480.0, style).unwrap();
        assert!(!text.resolve(&layout).unwrap().glyphs().is_empty());
        text.begin_epoch([&layout]);
        group.throughput(Throughput::Bytes(body.len() as u64));
        group.bench_function(BenchmarkId::new("warm", name), |b| {
            b.iter(|| black_box(text.layout(black_box(&body), 480.0, style).unwrap()));
        });
        let inputs: Vec<_> = (0..32).map(|i| format!("{i:02} {body}")).collect();
        for input in &inputs {
            text.begin_epoch(std::iter::empty());
            let _ = text.layout(input, 480.0, style).unwrap();
        }
        let mut index = 0;
        group.bench_function(BenchmarkId::new("miss_eviction", name), |b| {
            b.iter(|| {
                text.begin_epoch(std::iter::empty());
                let result = text
                    .layout(black_box(&inputs[index]), 480.0, style)
                    .unwrap();
                index = (index + 1) % inputs.len();
                black_box(result)
            });
        });
        println!(
            "TEXT_LAYOUT\t{name}\tlayouts={}\tlayout_bytes={}",
            text.stats().layouts,
            text.stats().layout_bytes
        );
    }
    group.finish();
}

fn rasters(c: &mut Criterion) {
    let mut group = c.benchmark_group("text/raster");
    for (name, capacity) in [("warm", 256), ("miss_eviction", 8)] {
        let mut text = system(TextBudgets {
            max_raster_glyphs: capacity,
            ..Default::default()
        });
        let layout = text.layout(BODY, 480.0, TextStyle::default()).unwrap();
        let mut unique = HashSet::new();
        let keys: Vec<_> = text
            .resolve(&layout)
            .unwrap()
            .glyphs()
            .iter()
            .map(|g| g.cache_key)
            .filter(|key| unique.insert(*key))
            .collect();
        assert!(keys.len() > 8 && keys.len() < 256);
        for &key in &keys {
            black_box(text.raster(key).unwrap());
        }
        group.throughput(Throughput::Elements(keys.len() as u64));
        group.bench_function(name, |b| {
            b.iter(|| {
                for &key in black_box(&keys) {
                    if let Some(glyph) = text.raster(key).unwrap() {
                        black_box(glyph.pixels);
                    }
                }
            });
        });
        println!(
            "TEXT_RASTER\t{name}\tkeys={}\traster_glyphs={}\traster_bytes={}",
            keys.len(),
            text.stats().raster_glyphs,
            text.stats().raster_bytes
        );
    }
    group.finish();
}

fn documents(c: &mut Criterion) {
    let body = BODY.repeat(16);
    let mut document = TextDocument::new(&body).unwrap();
    document
        .apply(TextOp::MoveSelection {
            movement: TextMovement::DocumentEnd,
            extend: false,
        })
        .unwrap();
    c.bench_function("text/edit_commit_delete_pair", |b| {
        b.iter(|| {
            document.apply(TextOp::Commit(black_box("界"))).unwrap();
            document
                .apply(TextOp::Delete(EditUnit::GraphemeBackward))
                .unwrap();
            black_box(document.text());
        });
    });
    assert_eq!(document.text(), body);
    c.bench_function("text/preedit_cancel_pair", |b| {
        b.iter(|| {
            document
                .apply(TextOp::Preedit {
                    text: black_box("输入拼音"),
                    selection: None,
                })
                .unwrap();
            black_box(document.composition());
            document.apply(TextOp::EndComposition).unwrap();
        });
    });
    let mut text = system(TextBudgets::default());
    let layout = document
        .layout(&mut text, 480.0, TextStyle::default())
        .unwrap();
    let points: Vec<_> = (0..32)
        .map(|i| Point::new((i * 47 % 480) as f32, (i * 23 % 400) as f32))
        .collect();
    c.bench_function("text/layout_hit_batch", |b| {
        b.iter(|| {
            for &point in black_box(&points) {
                black_box(text.resolve(&layout).unwrap().hit(point));
            }
        });
    });
    let selection = TextSelection {
        anchor: 0,
        focus: body.len(),
    };
    let mut rects = Vec::new();
    text.resolve(&layout)
        .unwrap()
        .selection_rects_into(selection, &mut rects);
    let rect_count = rects.len();
    c.bench_function("text/selection_geometry_reused", |b| {
        b.iter(|| {
            rects.clear();
            text.resolve(&layout)
                .unwrap()
                .selection_rects_into(black_box(selection), &mut rects);
            black_box(&rects);
        });
    });
    assert_eq!(rects.len(), rect_count);
}

criterion_group!(benches, layouts, rasters, documents);
criterion_main!(benches);
