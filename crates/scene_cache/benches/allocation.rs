//! 真实Cache事务包含目录准备、槽分配、发布和释放；碎片场景建立在计时外。
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use scene_cache::{Cache, Prepared};
use std::{hint::black_box, ops::Range, time::Duration};

struct Entry {
    range: Range<u32>,
}

fn fragmented(count: usize) -> Cache<Entry> {
    let mut cache = Cache::new().unwrap();
    let mut update = cache.begin(count).unwrap();
    let (slots, entries, _) = update.parts_mut();
    for _ in 0..count {
        entries.push(Prepared::Built(Entry {
            range: slots.take(4).unwrap(),
        }));
    }
    cache.commit(update, |entry| entry.range.clone()).unwrap();
    let mut update = cache.begin(count / 2).unwrap();
    for index in (1..count).step_by(2) {
        update.parts_mut().1.push(Prepared::Reused(index));
    }
    cache.commit(update, |entry| entry.range.clone()).unwrap();
    cache
}

fn allocation(c: &mut Criterion) {
    let mut group = c.benchmark_group("场景属性槽事务");
    group
        .sample_size(10)
        .warm_up_time(Duration::from_millis(100))
        .measurement_time(Duration::from_millis(200));
    for (count, dirty) in [(16, 4), (8192, 0), (8192, 32), (8192, 512)] {
        group.bench_function(format!("records{count}_new{dirty}"), |b| {
            b.iter_batched(
                || fragmented(count),
                |mut cache| {
                    let mut update = cache.begin(cache.entries().len() + dirty).unwrap();
                    let (slots, entries, _) = update.parts_mut();
                    for index in 0..cache.entries().len() {
                        entries.push(Prepared::Reused(index));
                    }
                    for _ in 0..dirty {
                        entries.push(Prepared::Built(Entry {
                            range: slots.take(3).unwrap(),
                        }));
                    }
                    cache.commit(update, |entry| entry.range.clone()).unwrap();
                    black_box(cache);
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}
criterion_group!(benches, allocation);
criterion_main!(benches);
