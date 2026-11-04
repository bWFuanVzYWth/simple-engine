//! 复用 UI 构建/发布与捕获拖动的 Criterion 基准，不含字体或 GPU。
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use input::{
    CommandKind, CommandRef, MonoTime, PointerCommand, PointerId, PointerPosition, Sequence,
    SourceId, Stamp,
};
use ui::{
    ActionId, BuildInfo, CancelBuffer, ControlStyle, EventBuffer, LayoutStyle, Limits, Rect,
    ScheduledVisual, SpaceEpoch, Ui, UiOp, UiStorage, ValueRange, ViewId, Widget, WidgetId,
};

const VIEW: ViewId = ViewId::new(1);
const SPACE: SpaceEpoch = SpaceEpoch::new(1);
const COLUMNS: usize = 32;

fn build(ui: &mut Ui, storage: &mut UiStorage, count: usize, cancels: &mut CancelBuffer) {
    let rows = count.div_ceil(COLUMNS);
    let width = COLUMNS as f32 * 24.0;
    let height = rows as f32 * 20.0;
    let mut builder = ui
        .begin(
            storage,
            BuildInfo::new(Rect::new(0.0, 0.0, width, height), SPACE),
        )
        .unwrap();
    let column = builder
        .column(builder.root(), LayoutStyle::new(width, height))
        .unwrap();
    for row in 0..rows {
        let parent = builder.row(column, LayoutStyle::new(width, 20.0)).unwrap();
        for index in row * COLUMNS..((row + 1) * COLUMNS).min(count) {
            builder
                .button(
                    parent,
                    Widget::new(
                        WidgetId::new(index as u128),
                        ActionId::new(index as u64),
                        "button",
                    ),
                    None,
                    ControlStyle {
                        size: [24.0, 20.0],
                        ..ControlStyle::default()
                    },
                )
                .unwrap();
        }
    }
    let candidate = builder.finish().unwrap();
    black_box(candidate.draw().shapes());
    let scheduled = ScheduledVisual::new(candidate.visual_id(), VIEW, SPACE);
    candidate.publish(scheduled, cancels).unwrap();
    cancels.clear();
}

fn build_publish(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("ui/build_layout_publish");
    for count in [64_usize, 512, 2048] {
        let limits = Limits {
            nodes: count + count.div_ceil(COLUMNS) + 2,
            pointers: 4,
            semantic_bytes: count * 16,
        };
        let mut ui = Ui::new(VIEW, limits).unwrap();
        let mut storage = UiStorage::new(limits).unwrap();
        let mut cancels = Vec::with_capacity(4);
        // 初始容量准备放在计时外；复用布局、绘制、身份索引和两份快照的后备。
        build(&mut ui, &mut storage, count, &mut cancels);
        build(&mut ui, &mut storage, count, &mut cancels);
        group.throughput(Throughput::Elements(count as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |bench, &count| {
                bench.iter(|| build(&mut ui, &mut storage, black_box(count), &mut cancels));
            },
        );
    }
    group.finish();
}

fn move_command(sequence: u64, x: f32) -> CommandRef<'static> {
    CommandRef {
        source: SourceId::new(1),
        stamp: Stamp {
            sequence: Sequence::new(sequence),
            received: MonoTime::new(sequence),
            sampled: None,
        },
        kind: CommandKind::Pointer(PointerCommand::Move {
            id: PointerId::new(1),
            at: PointerPosition {
                view: VIEW,
                space: SPACE,
                expected_snapshot: None,
                point: input::Point::new(x, 10.0),
            },
            pressure: None,
        }),
    }
}

fn slider_input(criterion: &mut Criterion) {
    let limits = Limits {
        nodes: 2,
        pointers: 1,
        semantic_bytes: 16,
    };
    let mut ui = Ui::new(VIEW, limits).unwrap();
    let mut storage = UiStorage::new(limits).unwrap();
    let mut builder = ui
        .begin(
            &mut storage,
            BuildInfo::new(Rect::new(0.0, 0.0, 200.0, 32.0), SPACE),
        )
        .unwrap();
    builder
        .slider(
            builder.root(),
            Widget::new(WidgetId::new(1), ActionId::new(1), "slider"),
            0.0,
            ValueRange::new(0.0, 100.0, 0.25).unwrap(),
            ControlStyle::default(),
        )
        .unwrap();
    let candidate = builder.finish().unwrap();
    let scheduled = ScheduledVisual::new(candidate.visual_id(), VIEW, SPACE);
    let mut cancels = Vec::with_capacity(2);
    candidate.publish(scheduled, &mut cancels).unwrap();
    let mut events: EventBuffer<'static> = EventBuffer::new();
    let mut down = move_command(1, 0.0);
    let CommandKind::Pointer(PointerCommand::Move { id, at, .. }) = down.kind else {
        unreachable!()
    };
    down.kind = CommandKind::Pointer(PointerCommand::Down {
        id,
        at,
        contact: input::Contact::Primary,
    });
    ui.route(down, &mut events, &mut cancels).unwrap();
    events.clear();
    cancels.clear();
    let mut sequence = 1_u64;
    let mut value = 0.0;
    let mut group = criterion.benchmark_group("ui/slider_route_apply");
    group.throughput(Throughput::Elements(256));
    // 从规范化命令到应用接受值，不含物理适配、绘制或文字；指针捕获持续有效。
    group.bench_function("256_moves", |bench| {
        bench.iter(|| {
            for index in 0..256 {
                sequence += 1;
                ui.route(
                    move_command(sequence, black_box(index as f32)),
                    &mut events,
                    &mut cancels,
                )
                .unwrap();
                for event in &events {
                    if let UiOp::SetValue(next) = event.op {
                        value = next;
                    }
                }
                events.clear();
            }
            black_box(value);
        });
    });
    group.finish();
}

criterion_group!(benches, build_publish, slider_input);
criterion_main!(benches);
