//! CPU 批内核覆盖。
//!
//! 输入生成、DAG 初态、native 描述和独立值 oracle 均在计时外；排序、去重和输出归位
//! 另列准备组。普通读复用 caller 输出；批写的 cold、warm、snapshot 明确区分池增长、
//! 高水位复用和旧版本保活。pair 每次迭代做两次调用并恢复相同终态，避免测成持续 noop。
//! `SVDAG_WORK` 记录每迭代输入元素数及统一 workload 入口调用数；Elements 统计输入项，
//! dense 统计体素，decode 的 Bytes 统计完整输出。资源日志仅计 DAG 显式保留容量；
//! 输入、caller 输出、同时保留的 Criterion 样本、线程栈、allocator 元数据和 RSS 另计。
//!
//! new/drop 包含实例分配与析构；drop_populated 的构建在 setup。first_growth 只测首次
//! root 槽增长和释放，release_unique 在 setup 分叉并编辑后测级联回收；后二者返回所有者，
//! 其结构移动计时、析构在计时外。LargeInput 批输入会影响缓存和总工作集。纯析构没有
//! 可读取终态；cold 对象由 Criterion 回收，仅单独预检，不伪造实测后的状态摘要。
//! 其他可观察状态输出计时前后摘要；射线还核对完整回调顺序、次数和输出。
//!
//! 内核均串行；本基准不覆盖 World 调度、mesher、GPU、GUI、allocator 调用次数或峰值 RSS。
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};

#[path = "coverage/current.rs"]
mod api;
#[path = "coverage/data.rs"]
mod data;
#[path = "coverage/lifecycle.rs"]
mod lifecycle;
#[path = "coverage/rays.rs"]
mod rays;
#[path = "coverage/read.rs"]
mod read;
#[path = "coverage/write.rs"]
mod write;

criterion_group! {
    name = coverage;
    config = Criterion::default()
        .sample_size(20)
        .warm_up_time(Duration::from_millis(200))
        .measurement_time(Duration::from_millis(500));
    targets = read::benches, write::benches, lifecycle::benches, rays::benches
}
criterion_main!(coverage);
