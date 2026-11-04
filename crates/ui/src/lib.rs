//! 面向已发布布局的即时 GUI；业务值由应用独占，输入统一经过 [`input`] 命令层。
//!
//! 一份 [`Ui`] 对应一个逻辑 view。应用按总序消费输入、修改唯一模型，再描述一次界面。
//! [`UiStorage`] 保留连续节点、绘制和候选快照；布局、命中与绘制共享同一份坐标和裁剪，
//! 不重跑业务描述，也不保存借用业务模型的回调。本 crate 不拥有窗口、GPU、音频或呈现循环。
//!
//! # 发布与失效
//!
//! [`Ui::begin`] 独占借用 UI 和存储，产生 [`UiBuilder`]，完成后取得 [`Candidate`]。
//! 仅在对应绘制已提交并排定呈现后，以 [`ScheduledVisual`] 发布；放弃候选保留原布局。
//! 发布不代表物理屏幕已经显示。候选存活期间无法再路由或构建，避免跨阶段隐含调用顺序。
//!
//! 几何、页面或交互能力变化须先 [`Ui::invalidate_interaction`] 并选择取消范围。
//! 新命中在布局无效时返回 [`RouteStatus::NeedsLayout`]，既不穿透业务，也不偷偷延迟到新布局；
//! 已捕获的释放仍清理原 owner。外部“打开菜单后操作新项”须等待新快照再发送第二条命令。
//!
//! # 文字与语义
//!
//! [`render2d::text::TextDocument`] 拥有正文与选区，UI 只保存发布时的布局与修订。
//! 编辑操作交还应用执行；旧布局命中必须由文档修订拒绝。外部修改当前焦点文档时调用
//! [`Ui::restart_text_session`] 并消费取消通知，防止迟到的旧 IME 提交写入新位置。
//! 单行输入关闭折行后横向跟随 caret；绘制、命中和 IME 坐标共享同一滚动偏移。
//! [`SemanticSnapshot`] 是只读发现接口，不替代平台完整的可访问性适配。
//!
//! # 构建与提交边界
//!
//! ```no_run
//! use ui::{
//!     ActionId, BuildInfo, ControlStyle, Limits, Rect, ScheduledVisual, SpaceEpoch,
//!     Ui, UiStorage, ViewId, Widget, WidgetId,
//! };
//!
//! let view = ViewId::new(1);
//! let space = SpaceEpoch::new(1);
//! let mut ui = Ui::new(view, Limits::default())?;
//! let mut storage = UiStorage::new(Limits::default())?;
//! let mut builder = ui.begin(&mut storage, BuildInfo::new(Rect::new(0.0, 0.0, 800.0, 600.0), space))?;
//! builder.button(
//!     builder.root(),
//!     Widget::new(WidgetId::new(1), ActionId::new(1), "运行"),
//!     None,
//!     ControlStyle::default(),
//! )?;
//! let candidate = builder.finish()?;
//! // host 在此提交 candidate.draw() 并排定呈现，然后才声明此视觉。
//! let scheduled = ScheduledVisual::new(candidate.visual_id(), view, space);
//! candidate.publish(scheduled, &mut Vec::new())?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
#![forbid(unsafe_code)]

mod build;
mod route;

pub use build::*;
pub use input::{ActionId, SnapshotId, SpaceEpoch, ViewId};
pub use render2d::{Color, Point, Rect};
pub use route::*;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
/// 应用分配的稳定业务键；重建时保持身份，不使用临时节点索引。
pub struct WidgetId(u128);

impl WidgetId {
    /// 包装应用稳定键；同一次构建的重复键由 builder 拒绝。
    pub const fn new(value: u128) -> Self {
        Self(value)
    }
    /// 适用于外部语义目标的原始稳定键。
    pub const fn get(self) -> u128 {
        self.0
    }
}

#[derive(Clone, Copy, Debug)]
/// 一份 UI 的显式节点、捕获和语义文本预算；各项必须非零。
pub struct Limits {
    /// 每次描述包含根容器在内的节点上限，不能超过 `u32::MAX`。
    pub nodes: usize,
    /// 同时持有的来源／指针／接触捕获槽上限。
    pub pointers: usize,
    /// 单次构建复制的 UTF-8 语义名称总字节上限。
    pub semantic_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            nodes: 4096,
            pointers: 32,
            semantic_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
/// 描述、布局或文字准备失败；失败候选不会替换已发布布局。
pub enum BuildError {
    /// 预算为零或节点数量超出可寻址范围。
    #[error("UI 预算必须非零且节点数量不能超过 u32::MAX")]
    InvalidLimits,
    /// 尺寸、间距、坐标或样式不是有效的有限几何。
    #[error("UI 布局参数非法")]
    InvalidLayout,
    /// 达到节点或语义名称的配置额度。
    #[error("UI 节点或语义文本预算已满")]
    Capacity,
    /// 节点来自其他 UI／构建、父节点不接受子节点，或偏移非法。
    #[error("父节点不属于本次构建或不能包含子节点")]
    InvalidParent,
    /// 同次描述重复使用稳定控件键。
    #[error("本次构建的控件身份重复：{0:?}")]
    DuplicateWidget(WidgetId),
    /// UI 所有者、构建或文字会话代次耗尽。
    #[error("UI 构建或文字会话代次已耗尽")]
    Generation,
    /// 数值范围、步长或 slider 当前值不合法。
    #[error("slider 范围、步长或值非法")]
    InvalidValue,
    /// 显式字体系统的文字准备失败，保留来源。
    #[error("文字准备失败：{0}")]
    Text(#[from] render2d::text::TextError),
    /// 最终有序场景校验失败，保留来源。
    #[error("绘制数据非法：{0}")]
    Scene(#[from] render2d::SceneError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use input::{
        CommandKind, CommandRef, Contact, MonoTime, PointerCommand, PointerId, PointerPosition,
        RequestId, SemanticOp, Sequence, SourceId, Stamp, Target, TargetVersion, TextOp,
    };
    use render2d::text::{TextBudgets, TextDocument, TextError, TextStyle, TextSystem};

    fn owners() -> (Ui, UiStorage) {
        (
            Ui::new(ViewId::new(1), Limits::default()).unwrap(),
            UiStorage::new(Limits::default()).unwrap(),
        )
    }
    fn info() -> BuildInfo {
        BuildInfo::new(Rect::new(0.0, 0.0, 500.0, 300.0), SpaceEpoch::new(1))
    }
    fn widget(id: u128) -> Widget<'static> {
        Widget::new(WidgetId::new(id), ActionId::new(id as u64), "control")
    }
    fn command<'a>(source: u64, kind: CommandKind<&'a str>) -> CommandRef<'a> {
        CommandRef {
            source: SourceId::new(source),
            stamp: Stamp {
                sequence: Sequence::new(1),
                received: MonoTime::new(1),
                sampled: None,
            },
            kind,
        }
    }
    fn pointer(source: u64, kind: u8, x: f32, y: f32) -> CommandRef<'static> {
        let id = PointerId::new(1);
        let at = PointerPosition {
            view: ViewId::new(1),
            space: SpaceEpoch::new(1),
            expected_snapshot: None,
            point: input::Point::new(x, y),
        };
        command(
            source,
            CommandKind::Pointer(match kind {
                0 => PointerCommand::Down {
                    id,
                    at,
                    contact: Contact::Primary,
                },
                1 => PointerCommand::Up {
                    id,
                    at,
                    contact: Contact::Primary,
                },
                _ => PointerCommand::Move {
                    id,
                    at,
                    pressure: None,
                },
            }),
        )
    }
    fn publish(candidate: Candidate<'_, '_>) {
        let scheduled =
            ScheduledVisual::new(candidate.visual_id(), ViewId::new(1), SpaceEpoch::new(1));
        candidate.publish(scheduled, &mut Vec::new()).unwrap();
    }
    fn button(ui: &mut Ui, storage: &mut UiStorage) {
        let mut build = ui.begin(storage, info()).unwrap();
        build
            .button(build.root(), widget(1), None, ControlStyle::default())
            .unwrap();
        publish(build.finish().unwrap());
    }
    fn invoke(op: SemanticOp<&str>, id: u128) -> CommandRef<'_> {
        command(
            1,
            CommandKind::Invoke {
                request: RequestId::new(1),
                target: Target {
                    view: ViewId::new(1),
                    widget: id,
                    version: TargetVersion::Current,
                },
                op,
            },
        )
    }

    #[test]
    fn short_press_activates_once() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let (mut events, mut cancels) = (EventBuffer::new(), Vec::new());
        ui.route(pointer(1, 0, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        ui.route(pointer(1, 1, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        assert!(
            matches!(events.as_slice(), [UiEvent { widget: id, op: UiOp::Activate, .. }] if *id == WidgetId::new(1))
        );
    }

    #[test]
    fn release_outside_does_not_activate() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let (mut events, mut cancels) = (EventBuffer::new(), Vec::new());
        ui.route(pointer(1, 0, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        assert_eq!(
            ui.route(pointer(1, 1, 400.0, 10.0), &mut events, &mut cancels)
                .unwrap(),
            RouteStatus::Handled
        );
        assert!(events.is_empty());
    }

    #[test]
    fn capture_identity_includes_source() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let (mut events, mut cancels) = (EventBuffer::new(), Vec::new());
        ui.route(pointer(1, 0, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        ui.route(pointer(2, 1, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        assert!(events.is_empty());
        ui.route(pointer(1, 1, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn captured_slider_clamps_outside_track() {
        let (mut ui, mut storage) = owners();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        build
            .slider(
                build.root(),
                widget(1),
                0.0,
                ValueRange::new(0.0, 10.0, 1.0).unwrap(),
                ControlStyle::default(),
            )
            .unwrap();
        publish(build.finish().unwrap());
        let (mut events, mut cancels) = (EventBuffer::new(), Vec::new());
        ui.route(pointer(1, 0, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        ui.route(pointer(1, 2, 400.0, 100.0), &mut events, &mut cancels)
            .unwrap();
        assert_eq!(events.last().unwrap().op, UiOp::SetValue(10.0));
    }

    #[test]
    fn invalidation_keeps_release_owned() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let (mut events, mut cancels) = (EventBuffer::new(), Vec::new());
        ui.route(pointer(1, 0, 10.0, 10.0), &mut events, &mut cancels)
            .unwrap();
        ui.invalidate_interaction(CancelScope::View, &mut cancels);
        assert_eq!(
            ui.route(pointer(1, 1, 400.0, 200.0), &mut events, &mut cancels)
                .unwrap(),
            RouteStatus::Handled
        );
        assert!(events.is_empty());
    }

    #[test]
    fn invalid_layout_blocks_new_hits() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        ui.invalidate_interaction(CancelScope::View, &mut Vec::new());
        assert_eq!(
            ui.route(
                pointer(1, 0, 10.0, 10.0),
                &mut EventBuffer::new(),
                &mut Vec::new()
            )
            .unwrap(),
            RouteStatus::NeedsLayout
        );
    }

    #[test]
    fn vanished_control_loses_focus_on_publication() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        ui.route(
            invoke(SemanticOp::Focus, 1),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let candidate = ui.begin(&mut storage, info()).unwrap().finish().unwrap();
        let scheduled =
            ScheduledVisual::new(candidate.visual_id(), ViewId::new(1), SpaceEpoch::new(1));
        let mut cancels = Vec::new();
        candidate.publish(scheduled, &mut cancels).unwrap();
        assert!(ui.focus().is_none());
        assert!(matches!(
            cancels.as_slice(),
            [UiCancel {
                kind: CancelKind::Focus,
                ..
            }]
        ));
    }

    #[test]
    fn abandoned_candidate_keeps_published_layout() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let old = ui.snapshot_id();
        {
            let _candidate = ui.begin(&mut storage, info()).unwrap().finish().unwrap();
        }
        assert_eq!(ui.snapshot_id(), old);
        assert_eq!(ui.semantics().unwrap().iter().count(), 1);
    }

    #[test]
    fn publication_requires_matching_visual() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let old = ui.snapshot_id();
        let candidate = ui.begin(&mut storage, info()).unwrap().finish().unwrap();
        let scheduled =
            ScheduledVisual::new(candidate.visual_id(), ViewId::new(2), SpaceEpoch::new(1));
        assert!(matches!(
            candidate.publish(scheduled, &mut Vec::new()),
            Err(PublishError::WrongVisual)
        ));
        assert_eq!(ui.snapshot_id(), old);
    }

    #[test]
    fn duplicate_widget_is_rejected() {
        let (mut ui, mut storage) = owners();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        build
            .button(build.root(), widget(1), None, ControlStyle::default())
            .unwrap();
        assert!(matches!(
            build.button(build.root(), widget(1), None, ControlStyle::default()),
            Err(BuildError::DuplicateWidget(_))
        ));
    }

    #[test]
    fn parent_clip_limits_child_hit_area() {
        let (mut ui, mut storage) = owners();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        let parent = build
            .absolute(build.root(), LayoutStyle::new(20.0, 20.0))
            .unwrap();
        build
            .button(parent, widget(1), None, ControlStyle::default())
            .unwrap();
        publish(build.finish().unwrap());
        assert_eq!(
            ui.route(
                pointer(1, 0, 30.0, 10.0),
                &mut EventBuffer::new(),
                &mut Vec::new()
            )
            .unwrap(),
            RouteStatus::Unclaimed
        );
    }

    #[test]
    fn node_from_previous_build_is_rejected() {
        let (mut ui, mut storage) = owners();
        let build = ui.begin(&mut storage, info()).unwrap();
        let root = build.root();
        publish(build.finish().unwrap());
        let mut build = ui.begin(&mut storage, info()).unwrap();
        assert!(matches!(
            build.button(root, widget(1), None, ControlStyle::default()),
            Err(BuildError::InvalidParent)
        ));
    }

    #[test]
    fn exact_semantic_version_rejects_old_snapshot() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        let old = ui.snapshot_id().unwrap();
        button(&mut ui, &mut storage);
        let command = command(
            1,
            CommandKind::Invoke {
                request: RequestId::new(1),
                target: Target {
                    view: ViewId::new(1),
                    widget: 1,
                    version: TargetVersion::ExactSnapshot(old),
                },
                op: SemanticOp::Activate,
            },
        );
        assert!(matches!(
            ui.route(command, &mut EventBuffer::new(), &mut Vec::new()),
            Err(RouteError::StaleSnapshot)
        ));
    }

    fn fonts() -> TextSystem {
        TextSystem::new(
            [include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/fonts/NotoSansSC-Regular.otf"
            ))
            .to_vec()],
            TextBudgets::default(),
        )
        .unwrap()
    }
    fn text_inputs(
        ui: &mut Ui,
        storage: &mut UiStorage,
        text: &mut TextSystem,
        document: &TextDocument,
    ) {
        let mut build = ui.begin(storage, info()).unwrap();
        let column = build
            .column(build.root(), LayoutStyle::new(240.0, 100.0))
            .unwrap();
        for id in 1..=2 {
            build
                .text_edit(
                    column,
                    widget(id),
                    document,
                    ControlStyle::default(),
                    TextStyle::default(),
                    text,
                )
                .unwrap();
        }
        publish(build.finish_with_text(text).unwrap());
    }

    #[test]
    fn current_and_candidate_layouts_survive_text_epoch_pressure() {
        let (mut ui, mut storage) = owners();
        let mut text = TextSystem::new(
            [include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/fonts/NotoSansSC-Regular.otf"
            ))
            .to_vec()],
            TextBudgets {
                max_layouts: 3,
                ..Default::default()
            },
        )
        .unwrap();
        text_inputs(
            &mut ui,
            &mut storage,
            &mut text,
            &TextDocument::new("当前").unwrap(),
        );
        let current = ui.text_layouts().next().unwrap().clone();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        build
            .text_edit(
                build.root(),
                widget(1),
                &TextDocument::new("候选").unwrap(),
                ControlStyle::default(),
                TextStyle::default(),
                &mut text,
            )
            .unwrap();
        let candidate = build.finish_with_text(&text).unwrap();
        for pressure in ["缓存甲", "缓存乙"] {
            text.begin_epoch(candidate.text_layouts());
            text.layout(pressure, 100.0, TextStyle::default()).unwrap();
        }
        assert!(text.resolve(&current).is_some());
        assert!(
            candidate
                .text_layouts()
                .all(|layout| text.resolve(layout).is_some())
        );
        publish(candidate);
        assert!(
            ui.text_layouts()
                .all(|layout| text.resolve(layout).is_some())
        );
    }

    #[test]
    fn old_ime_session_cannot_edit_new_focus() {
        let (mut ui, mut storage) = owners();
        let mut text = fonts();
        let document = TextDocument::new("中文").unwrap();
        text_inputs(&mut ui, &mut storage, &mut text, &document);
        ui.route_with_text(
            &text,
            invoke(SemanticOp::Focus, 1),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let old = ui.text_session().unwrap();
        ui.route_with_text(
            &text,
            invoke(SemanticOp::Focus, 2),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let mut events = EventBuffer::new();
        ui.route_with_text(
            &text,
            command(
                1,
                CommandKind::Text {
                    view: ViewId::new(1),
                    session: old,
                    edit: TextOp::Commit("误写"),
                },
            ),
            &mut events,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn scrolled_text_keeps_drawing_and_hit_coordinates_aligned() {
        let (mut ui, mut storage) = owners();
        let mut text = fonts();
        let mut document = TextDocument::new("用于单行输入的长文本").unwrap();
        document
            .apply(TextOp::MoveSelection {
                movement: input::TextMovement::DocumentEnd,
                extend: false,
            })
            .unwrap();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        build
            .text_edit(
                build.root(),
                widget(1),
                &document,
                ControlStyle {
                    size: [80.0, 40.0],
                    ..Default::default()
                },
                TextStyle {
                    wrap: false,
                    ..Default::default()
                },
                &mut text,
            )
            .unwrap();
        publish(build.finish_with_text(&text).unwrap());
        ui.route_with_text(
            &text,
            invoke(SemanticOp::Focus, 1),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let caret = ui.ime_caret(&text).unwrap();
        assert!(caret.x >= 6.0 && caret.right() <= 74.0);
        let mut events = EventBuffer::new();
        ui.route_with_text(
            &text,
            pointer(1, 0, caret.x + 0.25, caret.y + 1.0),
            &mut events,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(events.iter().any(
            |event| matches!(event.op, UiOp::TextHit { cursor, .. } if cursor == document.caret())
        ));
    }

    #[test]
    fn semantic_edit_rejects_an_old_ime_commit() {
        let (mut ui, mut storage) = owners();
        let mut text = fonts();
        let document = TextDocument::new("中文").unwrap();
        text_inputs(&mut ui, &mut storage, &mut text, &document);
        ui.route_with_text(
            &text,
            invoke(SemanticOp::Focus, 1),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let old = ui.text_session().unwrap();
        let mut events = EventBuffer::new();
        ui.route_with_text(
            &text,
            invoke(
                SemanticOp::Edit(TextOp::MoveSelection {
                    movement: input::TextMovement::DocumentEnd,
                    extend: false,
                }),
                1,
            ),
            &mut events,
            &mut Vec::new(),
        )
        .unwrap();
        events.clear();
        ui.route_with_text(
            &text,
            command(
                1,
                CommandKind::Text {
                    view: ViewId::new(1),
                    session: old,
                    edit: TextOp::Commit("误写"),
                },
            ),
            &mut events,
            &mut Vec::new(),
        )
        .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn text_hit_preserves_document_revision() {
        let (mut ui, mut storage) = owners();
        let mut text = fonts();
        let mut document = TextDocument::new("中文").unwrap();
        text_inputs(&mut ui, &mut storage, &mut text, &document);
        document.apply(TextOp::Commit("新")).unwrap();
        let mut events = EventBuffer::new();
        ui.route_with_text(
            &text,
            pointer(1, 0, 12.0, 12.0),
            &mut events,
            &mut Vec::new(),
        )
        .unwrap();
        let UiOp::TextHit {
            revision,
            cursor,
            extend,
        } = events[0].op
        else {
            panic!("expected text hit")
        };
        assert!(matches!(
            document.hit(revision, cursor, extend),
            Err(TextError::StaleRevision)
        ));
    }

    #[test]
    fn space_change_cancels_old_capture() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        ui.route(
            pointer(1, 0, 10.0, 10.0),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let mut changed = info();
        changed.space = SpaceEpoch::new(2);
        let mut build = ui.begin(&mut storage, changed).unwrap();
        build
            .button(build.root(), widget(1), None, ControlStyle::default())
            .unwrap();
        let candidate = build.finish().unwrap();
        let scheduled = ScheduledVisual::new(candidate.visual_id(), ViewId::new(1), changed.space);
        let mut cancels = Vec::new();
        candidate.publish(scheduled, &mut cancels).unwrap();
        assert!(!ui.pressed(WidgetId::new(1)));
        assert!(
            cancels
                .iter()
                .any(|cancel| cancel.kind == CancelKind::Capture)
        );
    }

    #[test]
    fn changed_action_does_not_inherit_focus() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        ui.route(
            invoke(SemanticOp::Focus, 1),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        build
            .button(
                build.root(),
                Widget::new(WidgetId::new(1), ActionId::new(9), "changed"),
                None,
                ControlStyle::default(),
            )
            .unwrap();
        publish(build.finish().unwrap());
        assert!(ui.focus().is_none());
    }

    #[test]
    fn changed_role_does_not_inherit_text_session() {
        let (mut ui, mut storage) = owners();
        let mut text = fonts();
        let document = TextDocument::new("文本").unwrap();
        text_inputs(&mut ui, &mut storage, &mut text, &document);
        ui.route_with_text(
            &text,
            invoke(SemanticOp::Focus, 1),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        button(&mut ui, &mut storage);
        assert!(ui.focus().is_none());
    }

    #[test]
    fn disabled_control_blocks_lower_hit() {
        let (mut ui, mut storage) = owners();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        build
            .button(build.root(), widget(1), None, ControlStyle::default())
            .unwrap();
        build
            .button(
                build.root(),
                widget(2),
                None,
                ControlStyle {
                    enabled: false,
                    ..ControlStyle::default()
                },
            )
            .unwrap();
        publish(build.finish().unwrap());
        let mut events = EventBuffer::new();
        ui.route(pointer(1, 0, 10.0, 10.0), &mut events, &mut Vec::new())
            .unwrap();
        ui.route(pointer(1, 1, 10.0, 10.0), &mut events, &mut Vec::new())
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn repeated_text_down_rotates_ime_session() {
        let (mut ui, mut storage) = owners();
        let mut text = fonts();
        let document = TextDocument::new("文本").unwrap();
        text_inputs(&mut ui, &mut storage, &mut text, &document);
        ui.route_with_text(
            &text,
            pointer(1, 0, 10.0, 10.0),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let old = ui.text_session().unwrap();
        ui.route_with_text(
            &text,
            pointer(1, 1, 10.0, 10.0),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        let mut cancels = Vec::new();
        ui.route_with_text(
            &text,
            pointer(1, 0, 20.0, 10.0),
            &mut EventBuffer::new(),
            &mut cancels,
        )
        .unwrap();
        assert_ne!(ui.text_session(), Some(old));
        assert!(
            cancels
                .iter()
                .any(|cancel| cancel.kind == CancelKind::TextSession)
        );
    }

    #[test]
    fn rejected_second_capture_keeps_its_release_owned() {
        let (mut ui, mut storage) = owners();
        button(&mut ui, &mut storage);
        ui.route(
            pointer(1, 0, 10.0, 10.0),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        ui.route(
            pointer(2, 0, 10.0, 10.0),
            &mut EventBuffer::new(),
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(
            ui.route(
                pointer(2, 1, 400.0, 200.0),
                &mut EventBuffer::new(),
                &mut Vec::new()
            )
            .unwrap(),
            RouteStatus::Handled
        );
    }

    #[test]
    fn node_from_another_ui_is_rejected() {
        let (mut first, mut first_storage) = owners();
        let (mut second, mut second_storage) = owners();
        let first_build = first.begin(&mut first_storage, info()).unwrap();
        let mut second_build = second.begin(&mut second_storage, info()).unwrap();
        assert!(matches!(
            second_build.button(first_build.root(), widget(1), None, ControlStyle::default()),
            Err(BuildError::InvalidParent)
        ));
    }
}
