//! 输入 arena 接收/消费与有界重放的 Criterion 基准，不含 OS 采集。
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use input::{
    ActionId, ActionPhase, ActionState, CommandBatch, CommandKind, Contact, Incoming, InputHub,
    InputItemRef, Limits, MonoTime, Navigation, Point, PointerCommand, PointerId, PointerPosition,
    Recorder, Recording, ReplayLimits, ReplayTime, RequestId, SampleTime, SemanticOp, SessionId,
    SourceId, SpaceEpoch, Target, TargetVersion, TextOp, TextSessionId, ViewId,
};
use std::hint::black_box;

#[derive(Clone, Copy)]
enum Pattern {
    Pointer,
    Editing,
}

fn prepare(out: &mut Vec<Incoming<'static>>, count: usize, start: u64, pattern: Pattern) {
    out.clear();
    for index in 0..count {
        let at = PointerPosition {
            view: ViewId::new(1),
            space: SpaceEpoch::new(1),
            expected_snapshot: None,
            point: Point::new(((start + index as u64) % 1024) as f32, index as f32 % 768.0),
        };
        let pointer = match index {
            0 => PointerCommand::Down {
                id: PointerId::new(1),
                at,
                contact: Contact::Primary,
            },
            end if end == count - 1 => PointerCommand::Up {
                id: PointerId::new(1),
                at,
                contact: Contact::Primary,
            },
            _ => PointerCommand::Move {
                id: PointerId::new(1),
                at,
                pressure: None,
            },
        };
        let action = ActionId::new((index / 8 % 16) as u64);
        let kind = match (pattern, index % 8) {
            (Pattern::Pointer, _) => CommandKind::Pointer(pointer),
            (_, 0) => CommandKind::Action {
                action,
                phase: ActionPhase::Press,
            },
            (_, 1) => CommandKind::Pointer(pointer),
            (_, 2) => CommandKind::Text {
                view: at.view,
                session: TextSessionId::new(1),
                edit: TextOp::Preedit {
                    text: "即时 GUI 🌈",
                    selection: None,
                },
            },
            (_, 3) => CommandKind::Action {
                action,
                phase: ActionPhase::Release,
            },
            (_, 4) => CommandKind::Text {
                view: at.view,
                session: TextSessionId::new(1),
                edit: TextOp::Commit("中文 / ASCII"),
            },
            (_, 5) => CommandKind::Navigate {
                view: at.view,
                direction: Navigation::Next,
            },
            (_, 6) => CommandKind::Scroll {
                at,
                delta: input::ScrollDelta::Pixels { x: 0.0, y: 16.0 },
            },
            _ => CommandKind::Invoke {
                request: RequestId::new(index as u128),
                target: Target {
                    view: at.view,
                    widget: 17,
                    version: TargetVersion::Current,
                },
                op: SemanticOp::SetValue((index % 100) as f64 / 100.0),
            },
        };
        let received = MonoTime::new(start + index as u64);
        out.push(Incoming {
            source: SourceId::new(1),
            received,
            sampled: Some(SampleTime {
                at: MonoTime::new(received.get() - 2),
                uncertainty_ns: 1,
            }),
            kind,
        });
    }
}

fn hub(count: usize) -> InputHub {
    let mut hub = InputHub::new(Limits {
        max_commands: count,
        max_text_bytes: count * 32,
        max_external_commands: 0,
        max_external_text_bytes: 0,
        max_sources: 1,
    })
    .unwrap();
    hub.register_source(SourceId::new(1)).unwrap();
    hub
}

fn consume(batch: &CommandBatch, state: &mut ActionState) -> u64 {
    let mut sum = 0u64;
    for item in batch.iter() {
        state.observe(item).unwrap();
        let InputItemRef::Command(command) = item else {
            unreachable!()
        };
        let value = match command.kind {
            CommandKind::Pointer(
                PointerCommand::Move { at, .. }
                | PointerCommand::Down { at, .. }
                | PointerCommand::Up { at, .. },
            ) => u64::from(at.point.x.to_bits() ^ at.point.y.to_bits()),
            CommandKind::Action { action, .. } => u64::from(state.is_pressed(action)),
            CommandKind::Text {
                edit: TextOp::Commit(text) | TextOp::Preedit { text, .. },
                ..
            } => text.bytes().map(u64::from).sum(),
            CommandKind::Invoke {
                op: SemanticOp::SetValue(value),
                ..
            } => value.to_bits(),
            _ => 1,
        };
        sum = sum.wrapping_add(value);
    }
    sum
}

fn batches(c: &mut Criterion) {
    let mut group = c.benchmark_group("input/batch");
    for (name, pattern) in [("pointer", Pattern::Pointer), ("editing", Pattern::Editing)] {
        for count in [16, 256, 4096] {
            let mut incoming = Vec::with_capacity(count);
            let mut hub = hub(count);
            let mut batch = CommandBatch::with_capacity(count, count * 32).unwrap();
            let mut state = ActionState::with_capacity(16).unwrap();
            let mut start = 100;
            group.throughput(Throughput::Elements(count as u64));
            group.bench_function(BenchmarkId::new(name, count), |b| {
                b.iter(|| {
                    prepare(&mut incoming, count, start, pattern);
                    start += count as u64;
                    for &event in black_box(incoming.as_slice()) {
                        hub.push(event).unwrap();
                    }
                    hub.drain_into(&mut batch).unwrap();
                    black_box(consume(&batch, &mut state));
                })
            });
        }
    }
    group.finish();
}

fn recording(count: usize) -> Recording {
    let mut incoming = Vec::with_capacity(count);
    prepare(&mut incoming, count, 100, Pattern::Editing);
    let mut hub = hub(count);
    for event in incoming {
        hub.push(event).unwrap();
    }
    let mut batch = CommandBatch::new();
    hub.drain_into(&mut batch).unwrap();
    let mut recorder = Recorder::new(SessionId::new(1), MonoTime::new(0));
    recorder.append(&batch).unwrap();
    recorder.finish()
}

fn replays(c: &mut Criterion) {
    let count = 8192;
    let recording = recording(count);
    let mut group = c.benchmark_group("input/typed_replay");
    group.throughput(Throughput::Elements(count as u64));
    for chunk in [256, 4096] {
        let limits = ReplayLimits {
            max_items: chunk,
            max_text_bytes: chunk * 32,
        };
        let mut batch = CommandBatch::with_capacity(chunk, limits.max_text_bytes).unwrap();
        let mut state = ActionState::with_capacity(16).unwrap();
        group.bench_function(BenchmarkId::new("editing", chunk), |b| {
            b.iter(|| {
                let mut replay = recording
                    .replay_with_limits(MonoTime::new(1000), limits)
                    .unwrap();
                loop {
                    batch.clear();
                    let progress = replay
                        .read_until(ReplayTime::new(100 + count as u64), &mut batch)
                        .unwrap();
                    black_box(consume(&batch, &mut state));
                    if progress.finished {
                        break;
                    }
                }
            })
        });
    }
    group.finish();
}

criterion_group!(benches, batches, replays);
criterion_main!(benches);
