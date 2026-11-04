//! 只在已发布快照上路由输入；焦点、来源捕获和 IME 会话由同一个 [`Ui`] 所有者维护。

use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

use input::{
    ActionPhase, CommandKind, CommandRef, Contact, InputItemRef, InputScope, Navigation,
    PointerCommand, PointerId, PointerPosition, RequestId, SemanticOp, SourceId, Stamp,
    TargetVersion, TextOpRef, TextRevision, TextSessionId,
};
use render2d::text::{TextCursor, TextLayout, TextSystem};

use crate::{ActionId, BuildError, Limits, Point, Rect, SnapshotId, SpaceEpoch, ViewId, WidgetId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 上层决定是否继续分发或先重建布局的路由结果。
pub enum RouteStatus {
    /// 已被 UI 消费，即使未产生业务操作也不能继续穿透。
    Handled,
    /// 不属于该 UI，可交给其他消费者。
    Unclaimed,
    /// 新命中需要有效快照；应重建后显式重试，不能穿透或隐式延期。
    NeedsLayout,
}

#[derive(Debug, thiserror::Error)]
/// 路由目标、版本或持续状态预算失败。
pub enum RouteError {
    /// 指定坐标空间、快照或文档修订不匹配。
    #[error("目标快照或坐标空间已过期")]
    StaleSnapshot,
    /// 目标不存在或已禁用。
    #[error("目标控件不存在或已禁用")]
    TargetGone,
    /// 目标角色不接受请求的操作。
    #[error("控件不支持此操作")]
    Unsupported,
    /// 同时捕获的来源／指针／接触组合已满。
    #[error("指针捕获预算已满")]
    PointerLimit,
    /// 新 IME 会话代次无法再递增。
    #[error("文字会话代次已耗尽")]
    Generation,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 交还应用顺序执行的业务操作，UI 不直接修改模型。
pub enum UiOp<'a> {
    /// 执行按钮或开关的主要动作。
    Activate,
    /// 设置拖动条已裁剪、量化的绝对值。
    SetValue(f64),
    /// 按控件步长增减；应用以最新业务值计算结果。
    Step(i32),
    /// 对应用独占的文档执行编辑；文本借用原输入批次。
    Edit(TextOpRef<'a>),
    /// 用已发布布局的命中结果移动文档光标；应用必须校验修订。
    TextHit {
        /// 产生此命中的文档修订。
        revision: TextRevision,
        /// 正文 UTF-8 字节位置；应用通过文档校验并对齐 grapheme 边界。
        cursor: TextCursor,
        /// 保留原锚点以扩选，否则收拢选区。
        extend: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 带原始分发证据的控件操作；编辑载荷不能越过输入批次生命周期。
pub struct UiEvent<'a> {
    /// 原始命令的时间与总序。
    pub stamp: Stamp,
    /// 发出该操作的逻辑适配槽。
    pub source: SourceId,
    /// 已发布布局中的稳定控件键。
    pub widget: WidgetId,
    /// 控件描述时绑定的业务动作。
    pub action: ActionId,
    /// 需要应用执行的操作。
    pub op: UiOp<'a>,
    /// 仅语义请求保留相关身份；UI 不提供请求去重。
    pub request: Option<RequestId>,
}

/// 调用者复用的操作输出；两条以内内联保存，更多操作可扩容。
pub type EventBuffer<'a> = smallvec::SmallVec<[UiEvent<'a>; 2]>;
/// 调用者复用的取消通知输出；各接口追加而不隐式清空。
pub type CancelBuffer = Vec<UiCancel>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// host 与应用须清理的持续交互类型。
pub enum CancelKind {
    /// 捕获被取消，不应据此触发正常点击。
    Capture,
    /// 焦点丢失，应结束该控件的业务组合态。
    Focus,
    /// 仅取消平台旧 IME 会话；模型组合态由同次编辑处理，不能提前结束使命中修订失效。
    TextSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 持续 owner 被取消的通知；保留旧动作与来源供应用清理。
pub struct UiCancel {
    /// 失去交互所有权的稳定控件键。
    pub widget: WidgetId,
    /// 原先绑定的业务动作，不使用新布局重新解释。
    pub action: ActionId,
    /// 创建焦点或捕获的来源；并非控件输入权限令牌。
    pub source: SourceId,
    /// 平台和业务需要清理的状态类型。
    pub kind: CancelKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 使布局失效时立即取消哪些现有 owner；新命中均须等待重新发布。
pub enum CancelScope {
    /// 保留现有 owner，仅适用于不改变其几何与交互含义的更新。
    None,
    /// 取消指定控件的焦点与捕获。
    Widget(WidgetId),
    /// 页面、模态或坐标映射改变时取消整个 view。
    View,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 有限闭区间与正步长；步进从最小值开始，最大值始终允许精确到达。
pub struct ValueRange {
    min: f64,
    max: f64,
    step: f64,
}

impl ValueRange {
    /// 校验 `min < max`、正步长及中间算术范围，非法返回 [`BuildError::InvalidValue`]。
    pub fn new(min: f64, max: f64, step: f64) -> Result<Self, BuildError> {
        if !min.is_finite()
            || !max.is_finite()
            || !(max - min).is_finite()
            || !step.is_finite()
            || min >= max
            || step <= 0.0
            || !((max - min) / step).is_finite()
        {
            return Err(BuildError::InvalidValue);
        }
        Ok(Self { min, max, step })
    }
    /// 可到达的最小值，也是步进网格原点。
    pub const fn min(self) -> f64 {
        self.min
    }
    /// 可到达的最大值，不要求恰好落在步进网格上。
    pub const fn max(self) -> f64 {
        self.max
    }
    /// 逻辑单位的正步长。
    pub const fn step(self) -> f64 {
        self.step
    }
    /// 将值裁剪到闭区间并按最近步长吸附；调用方须先排除 NaN。
    /// 中点采用 [`f64::round`] 规则，两端保留精确端点。
    pub fn snap(self, value: f64) -> f64 {
        if value <= self.min {
            return self.min;
        }
        if value >= self.max {
            return self.max;
        }
        (((value.clamp(self.min, self.max) - self.min) / self.step).round() * self.step + self.min)
            .clamp(self.min, self.max)
    }
    /// 吸附后相对闭区间的位置，位于 `[0, 1]`；调用方须先排除 NaN。
    pub fn fraction(self, value: f64) -> f32 {
        ((self.snap(value) - self.min) / (self.max - self.min)) as f32
    }
    fn at(self, bounds: Rect, x: f32) -> f64 {
        let fraction = if bounds.width > 0.0 {
            ((x - bounds.x) / bounds.width).clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.snap(f64::from(fraction) * (self.max - self.min) + self.min)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 语义发现与能力检查使用的控件角色，不暴露内部布局节点。
pub enum Role {
    /// 主要操作可被激活。
    Button,
    /// 当前值为开关状态，激活由应用翻转。
    Toggle,
    /// 支持绝对值和步进的有界数值控件。
    Slider,
    /// 支持文本编辑与 IME 焦点。
    TextInput,
}

#[derive(Clone, Copy, Debug, PartialEq)]
/// 发布时可发现的只读业务值；文本正文由应用另行提供。
pub enum SemanticValue {
    /// 无需公开额外值，如普通按钮。
    None,
    /// 开关发布时的选中状态。
    Toggle(bool),
    /// 数值控件发布时的当前值。
    Number(f64),
    /// 文本布局对应的文档修订，不包含正文副本。
    TextRevision(TextRevision),
}

/// 已发布控件的只读语义视图；借用到快照被替换前。
pub struct SemanticNode<'a> {
    /// 供语义命令定位的稳定业务键。
    pub widget: WidgetId,
    /// 发布时绑定的业务动作。
    pub action: ActionId,
    /// 决定支持操作的控件角色。
    pub role: Role,
    /// 构建时复制的 UTF-8 名称。
    pub name: &'a str,
    /// 完整逻辑边界，可能超出祖先裁剪。
    pub bounds: Rect,
    /// 边界与祖先裁剪的交集，可为空。
    pub visible_bounds: Rect,
    /// 是否允许建立新的交互 owner。
    pub enabled: bool,
    /// 发布时的只读值，不代表尚未重建的当前模型。
    pub value: SemanticValue,
    /// 仅 slider 提供合法范围与步长。
    pub range: Option<ValueRange>,
}

/// 对一份不可变布局的语义借用；可用于发现，不自动建立平台可访问性树。
pub struct SemanticSnapshot<'a> {
    pub(crate) snapshot: &'a InteractionSnapshot,
}

impl SemanticSnapshot<'_> {
    /// 用于外部请求精确版本约束的快照身份。
    pub fn id(&self) -> SnapshotId {
        self.snapshot.id
    }
    /// 此快照所属逻辑视图。
    pub fn view(&self) -> ViewId {
        self.snapshot.view
    }
    /// 所有边界采用的坐标空间代次。
    pub const fn space(&self) -> SpaceEpoch {
        self.snapshot.space
    }
    /// 按描述顺序借用控件，包含禁用或完全被裁剪的节点。
    pub fn iter(&self) -> impl ExactSizeIterator<Item = SemanticNode<'_>> {
        self.snapshot.behaviors.iter().map(|behavior| SemanticNode {
            widget: behavior.widget,
            action: behavior.action,
            role: behavior.role(),
            name: &self.snapshot.names[behavior.name.clone()],
            bounds: behavior.bounds,
            visible_bounds: behavior.bounds.intersection(behavior.clip),
            enabled: behavior.enabled,
            value: behavior.value,
            range: match behavior.kind {
                BehaviorKind::Slider(range) => Some(range),
                _ => None,
            },
        })
    }
}

pub(crate) enum BehaviorKind {
    Button,
    Toggle,
    Slider(ValueRange),
    Text {
        revision: TextRevision,
        layout: TextLayout,
        origin: Point,
        caret: TextCursor,
        scroll_x: f32,
        viewport: Rect,
    },
}

pub(crate) struct Behavior {
    pub widget: WidgetId,
    pub action: ActionId,
    pub bounds: Rect,
    pub clip: Rect,
    pub enabled: bool,
    pub name: std::ops::Range<usize>,
    pub value: SemanticValue,
    pub kind: BehaviorKind,
}

impl Behavior {
    pub(crate) fn role(&self) -> Role {
        match self.kind {
            BehaviorKind::Button => Role::Button,
            BehaviorKind::Toggle => Role::Toggle,
            BehaviorKind::Slider(_) => Role::Slider,
            BehaviorKind::Text { .. } => Role::TextInput,
        }
    }
    fn contains(&self, point: Point) -> bool {
        self.bounds.contains(point) && self.clip.contains(point)
    }
    fn event<'a>(&self, command: CommandRef<'a>, op: UiOp<'a>) -> UiEvent<'a> {
        UiEvent {
            stamp: command.stamp,
            source: command.source,
            widget: self.widget,
            action: self.action,
            op,
            request: match command.kind {
                CommandKind::Invoke { request, .. } => Some(request),
                _ => None,
            },
        }
    }
    fn point_op(
        &self,
        point: Point,
        extend: bool,
        text: Option<&TextSystem>,
    ) -> Option<UiOp<'static>> {
        match &self.kind {
            BehaviorKind::Slider(range) => Some(UiOp::SetValue(range.at(self.bounds, point.x))),
            BehaviorKind::Text {
                revision,
                layout,
                origin,
                ..
            } => text
                .and_then(|owner| owner.resolve(layout))?
                .hit(Point::new(point.x - origin.x, point.y - origin.y))
                .map(|cursor| UiOp::TextHit {
                    revision: *revision,
                    cursor,
                    extend,
                }),
            _ => None,
        }
    }
}

pub(crate) struct InteractionSnapshot {
    pub id: SnapshotId,
    pub view: ViewId,
    pub space: SpaceEpoch,
    pub discover: bool,
    pub names: String,
    pub behaviors: Vec<Behavior>,
    pub indices: HashMap<WidgetId, usize>,
}

impl InteractionSnapshot {
    pub fn new(view: ViewId, capacity: usize, text_bytes: usize) -> Self {
        Self {
            id: SnapshotId::new(0),
            view,
            space: SpaceEpoch::new(0),
            discover: true,
            names: String::with_capacity(text_bytes),
            behaviors: Vec::with_capacity(capacity),
            indices: HashMap::with_capacity(capacity),
        }
    }
    pub fn clear(&mut self) {
        self.behaviors.clear();
        self.names.clear();
        self.indices.clear();
    }
    fn behavior(&self, widget: WidgetId) -> Option<&Behavior> {
        self.indices
            .get(&widget)
            .and_then(|index| self.behaviors.get(*index))
            .filter(|behavior| behavior.enabled)
    }
}

#[derive(Clone, Copy)]
struct Focus {
    widget: WidgetId,
    action: ActionId,
    source: SourceId,
    role: Role,
}

#[derive(Clone, Copy)]
struct Capture {
    source: SourceId,
    pointer: PointerId,
    contact: Contact,
    target: Option<(WidgetId, ActionId)>,
    role: Role,
}

/// 单个逻辑 view 的已发布快照、焦点与捕获所有者。
///
/// 不持有业务模型或平台对象。输入只在已发布布局上路由；同一焦点的文本会话供所有来源
/// 共用，`SourceId` 仅区分持续状态 owner，不作为编辑权限。新会话会拒绝旧平台 IME 提交。
/// 捕获身份包含来源、指针和接触通道，不能因另一来源释放而结束。
pub struct Ui {
    pub(crate) owner: u64,
    pub(crate) view: ViewId,
    pub(crate) snapshot: InteractionSnapshot,
    pub(crate) valid: bool,
    pub(crate) initialized: bool,
    pub(crate) limits: Limits,
    pub(crate) next_build: u64,
    captures: Vec<Capture>,
    focus: Option<Focus>,
    text_session: u64,
    activate_action: Option<ActionId>,
    hover: Option<WidgetId>,
}

impl Ui {
    /// 建立尚无有效布局的 view 并预留交互容量。
    /// 非法预算或全局所有者代次耗尽时失败；首个新命中需先构建并发布。
    pub fn new(view: ViewId, limits: Limits) -> Result<Self, BuildError> {
        if limits.nodes == 0
            || limits.nodes > u32::MAX as usize
            || limits.pointers == 0
            || limits.semantic_bytes == 0
        {
            return Err(BuildError::InvalidLimits);
        }
        static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);
        let owner = NEXT_OWNER
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| BuildError::Generation)?;
        Ok(Self {
            owner,
            view,
            snapshot: InteractionSnapshot::new(view, limits.nodes, limits.semantic_bytes),
            valid: false,
            initialized: false,
            limits,
            next_build: 1,
            captures: Vec::with_capacity(limits.pointers),
            focus: None,
            text_session: 0,
            activate_action: None,
            hover: None,
        })
    }

    /// 绑定焦点控件的逻辑激活动作；只消费 Press，`None` 关闭此映射。
    pub fn set_activation_action(&mut self, action: Option<ActionId>) {
        self.activate_action = action;
    }
    /// 当前焦点的稳定键，失效取消或后续发布可能清除它。
    pub fn focus(&self) -> Option<WidgetId> {
        self.focus.map(|focus| focus.widget)
    }
    /// 最近一次有效指针路由命中的控件；布局失效时立即清除。
    pub fn hovered(&self) -> Option<WidgetId> {
        self.hover
    }
    /// 是否仍有指针接触捕获此控件；不是业务值，也不代表当前 hover。
    pub fn pressed(&self, widget: WidgetId) -> bool {
        self.captures
            .iter()
            .any(|capture| capture.target.is_some_and(|target| target.0 == widget))
    }
    /// 当前文本焦点的 IME 会话；非文本焦点或已失去目标时为 `None`。
    /// 平台须在组合开始时绑定代次，不能把迟到提交重新绑定到当前会话。
    pub fn text_session(&self) -> Option<TextSessionId> {
        let focus = self.focus?;
        matches!(
            self.snapshot.behavior(focus.widget)?.kind,
            BehaviorKind::Text { .. }
        )
        .then_some(TextSessionId::new(self.text_session))
    }
    /// 应用从其他控件改正文／光标时，取消该焦点的旧平台 IME 会话。
    ///
    /// 仅当前焦点文本控件会递增会话并追加 [`CancelKind::TextSession`]；其他目标无操作。
    /// 不直接结束模型的 composition，应用按同次编辑语义处理；代次耗尽时失败。
    pub fn restart_text_session(
        &mut self,
        widget: WidgetId,
        out: &mut CancelBuffer,
    ) -> Result<(), RouteError> {
        let Some(focus) = self
            .focus
            .filter(|focus| focus.widget == widget && focus.role == Role::TextInput)
        else {
            return Ok(());
        };
        self.text_session = self
            .text_session
            .checked_add(1)
            .ok_or(RouteError::Generation)?;
        out.push(UiCancel {
            widget,
            action: focus.action,
            source: focus.source,
            kind: CancelKind::TextSession,
        });
        Ok(())
    }
    /// 当前发布快照的文字身份；新文字代次须保留这些身份直到替换快照。
    pub fn text_layouts(&self) -> impl Iterator<Item = &TextLayout> {
        self.snapshot
            .behaviors
            .iter()
            .filter_map(|behavior| match &behavior.kind {
                BehaviorKind::Text { layout, .. } => Some(layout),
                _ => None,
            })
    }

    /// 有效已发布布局中的 caret 逻辑矩形，已包含滚动与内边距裁剪。
    ///
    /// 无文本焦点、布局无效或 caret 完全不可见时为 `None`。
    /// 正文更新后，重建并发布前仍是旧位置；不能用它查询尚未发布候选。
    pub fn ime_caret(&self, text: &TextSystem) -> Option<Rect> {
        if !self.valid {
            return None;
        }
        let behavior = self.snapshot.behavior(self.focus?.widget)?;
        let BehaviorKind::Text {
            layout,
            origin,
            caret,
            viewport,
            ..
        } = &behavior.kind
        else {
            return None;
        };
        let caret = text.resolve(layout)?.caret_rect(*caret)?;
        let rect = Rect::new(
            caret.x + origin.x,
            caret.y + origin.y,
            caret.width.max(1.0),
            caret.height,
        )
        .intersection(*viewport);
        (!rect.is_empty()).then_some(rect)
    }
    /// 借用最后发布的语义快照；尚未发布或关闭语义发现时为 `None`。
    /// 失效不会删除旧快照，消费者须结合 [`Self::needs_layout`] 判断是否可操作。
    pub fn semantics(&self) -> Option<SemanticSnapshot<'_>> {
        (self.initialized && self.snapshot.discover).then_some(SemanticSnapshot {
            snapshot: &self.snapshot,
        })
    }
    /// 最后发布的快照身份；失效后仍保留，不代表当前布局可用于新命中。
    pub fn snapshot_id(&self) -> Option<SnapshotId> {
        self.initialized.then_some(self.snapshot.id)
    }
    /// 新命中是否必须等待一次有效发布；已捕获的释放和取消仍应路由。
    pub const fn needs_layout(&self) -> bool {
        !self.valid
    }

    /// 立即使新命中失效并追加所选 owner 的取消通知，保留旧快照供收尾。
    ///
    /// [`CancelScope::None`] 仅用于不影响已有 owner 几何的更新；页面或模态替换使用 View。
    /// 被取消的捕获保留释放归属，后续 Up 仍被 UI 消费，避免穿透其他消费者。
    pub fn invalidate_interaction(&mut self, scope: CancelScope, out: &mut CancelBuffer) {
        self.valid = false;
        self.hover = None;
        self.cancel_matching(
            |widget, _| match scope {
                CancelScope::None => false,
                CancelScope::Widget(target) => widget == target,
                CancelScope::View => true,
            },
            out,
        );
    }

    fn cancel_matching(
        &mut self,
        matches: impl Fn(WidgetId, SourceId) -> bool,
        out: &mut CancelBuffer,
    ) {
        for capture in &mut self.captures {
            if let Some((widget, action)) = capture.target
                && matches(widget, capture.source)
            {
                out.push(UiCancel {
                    widget,
                    action,
                    source: capture.source,
                    kind: CancelKind::Capture,
                });
                // 保留手势墓碑直到 Up，释放不能穿透到其他消费者。
                capture.target = None;
            }
        }
        if let Some(focus) = self.focus
            && matches(focus.widget, focus.source)
        {
            out.push(UiCancel {
                widget: focus.widget,
                action: focus.action,
                source: focus.source,
                kind: CancelKind::Focus,
            });
            self.focus = None;
        }
    }

    pub(crate) fn reconcile(&mut self, out: &mut CancelBuffer) {
        let snapshot = &self.snapshot;
        for capture in &mut self.captures {
            if let Some((widget, action)) = capture.target
                && snapshot.behavior(widget).is_none_or(|behavior| {
                    behavior.action != action || behavior.role() != capture.role
                })
            {
                out.push(UiCancel {
                    widget,
                    action,
                    source: capture.source,
                    kind: CancelKind::Capture,
                });
                capture.target = None;
            }
        }
        if let Some(focus) = self.focus
            && snapshot.behavior(focus.widget).is_none_or(|behavior| {
                behavior.action != focus.action || behavior.role() != focus.role
            })
        {
            out.push(UiCancel {
                widget: focus.widget,
                action: focus.action,
                source: focus.source,
                kind: CancelKind::Focus,
            });
            self.focus = None;
        }
    }

    fn set_focus(
        &mut self,
        widget: WidgetId,
        source: SourceId,
        out: &mut CancelBuffer,
    ) -> Result<(), RouteError> {
        if self
            .focus
            .is_some_and(|focus| focus.widget == widget && focus.source == source)
        {
            return Ok(());
        }
        let next = self
            .text_session
            .checked_add(1)
            .ok_or(RouteError::Generation)?;
        if let Some(old) = self.focus.take() {
            out.push(UiCancel {
                widget: old.widget,
                action: old.action,
                source: old.source,
                kind: CancelKind::Focus,
            });
        }
        let behavior = self
            .snapshot
            .behavior(widget)
            .ok_or(RouteError::TargetGone)?;
        self.focus = Some(Focus {
            widget,
            action: behavior.action,
            source,
            role: behavior.role(),
        });
        self.text_session = next;
        Ok(())
    }

    /// 路由批次事件，缺口取消对应来源的焦点／捕获并被消费。
    ///
    /// 命令行为与错误同 [`Self::route`]；输出只追加，调用者决定何时清空复用缓冲。
    pub fn route_item<'a>(
        &mut self,
        item: InputItemRef<'a>,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
    ) -> Result<RouteStatus, RouteError> {
        match item {
            InputItemRef::Command(command) => self.route(command, events, cancels),
            InputItemRef::Gap(gap) => {
                self.cancel_source(gap.source, cancels);
                Ok(RouteStatus::Handled)
            }
        }
    }

    /// 同 route_item，并借用文字所有者处理文本命中。
    pub fn route_item_with_text<'a>(
        &mut self,
        text: &TextSystem,
        item: InputItemRef<'a>,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
    ) -> Result<RouteStatus, RouteError> {
        match item {
            InputItemRef::Command(command) => self.route_with_text(text, command, events, cancels),
            InputItemRef::Gap(gap) => {
                self.cancel_source(gap.source, cancels);
                Ok(RouteStatus::Handled)
            }
        }
    }

    fn cancel_source(&mut self, source: SourceId, cancels: &mut CancelBuffer) {
        self.cancel_matching(|_, owner| owner == source, cancels);
        self.captures.retain(|capture| capture.source != source);
    }

    /// 在已发布布局上消费一条已校验命令，追加操作及取消通知，不修改业务模型。
    ///
    /// 无效布局的新命中返回 [`RouteStatus::NeedsLayout`]，调用者不能让其穿透业务。
    /// 捕获释放先清理原 owner，过期坐标也不会留下持续按下；不支持的视图返回 Unclaimed。
    /// 旧 IME 会话被消费但不产生编辑；语义 Edit 会先发出操作，再使原 IME 会话失效。
    ///
    /// 过期坐标／目标版本、失去或禁用的目标、不支持的操作、捕获超额及代次耗尽返回错误。
    /// 文档版本这里只能对照发布布局，应用仍须校验尚未发布的当前文档修订。
    /// 输出缓冲只追加，编辑文本借用输入载荷；调用方须先通过 [`input::InputHub`] 校验命令。
    /// 失败不保证回滚已更新的交互状态或已追加的通知，调用者仍须处理输出前缀。
    pub fn route<'a>(
        &mut self,
        command: CommandRef<'a>,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
    ) -> Result<RouteStatus, RouteError> {
        self.route_impl(command, events, cancels, None)
    }
    /// 使用文字所有者解析已发布布局中的命中；不复制布局负载。
    pub fn route_with_text<'a>(
        &mut self,
        text: &TextSystem,
        command: CommandRef<'a>,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
    ) -> Result<RouteStatus, RouteError> {
        self.route_impl(command, events, cancels, Some(text))
    }
    fn route_impl<'a>(
        &mut self,
        command: CommandRef<'a>,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
        text: Option<&TextSystem>,
    ) -> Result<RouteStatus, RouteError> {
        match command.kind {
            CommandKind::Pointer(pointer) => self.pointer(command, pointer, events, cancels, text),
            CommandKind::Cancel { scope, .. } => {
                match scope {
                    InputScope::Source => self.cancel_source(command.source, cancels),
                    InputScope::All => {
                        self.cancel_matching(|_, _| true, cancels);
                        self.captures.clear();
                    }
                    InputScope::View(view) if view == self.view => {
                        self.cancel_matching(|_, _| true, cancels);
                        self.captures.clear();
                    }
                    InputScope::View(_) => return Ok(RouteStatus::Unclaimed),
                }
                Ok(RouteStatus::Handled)
            }
            CommandKind::Text {
                view,
                session,
                edit,
            } if view == self.view => {
                if self.text_session() != Some(session) {
                    return Ok(RouteStatus::Handled);
                }
                let focus = self.focus.ok_or(RouteError::TargetGone)?;
                let behavior = self
                    .snapshot
                    .behavior(focus.widget)
                    .ok_or(RouteError::TargetGone)?;
                events.push(behavior.event(command, UiOp::Edit(edit)));
                Ok(RouteStatus::Handled)
            }
            CommandKind::Invoke { target, op, .. } if target.view == self.view => {
                if !self.valid {
                    return Ok(RouteStatus::NeedsLayout);
                }
                let widget = WidgetId::new(target.widget);
                let behavior = self
                    .snapshot
                    .behavior(widget)
                    .ok_or(RouteError::TargetGone)?;
                match target.version {
                    TargetVersion::ExactSnapshot(id) if id != self.snapshot.id => {
                        return Err(RouteError::StaleSnapshot);
                    }
                    TargetVersion::Document(revision) if !matches!(behavior.kind, BehaviorKind::Text { revision: current, .. } if revision == current) =>
                    {
                        return Err(RouteError::StaleSnapshot);
                    }
                    _ => {}
                }
                if op == SemanticOp::Focus {
                    self.set_focus(widget, command.source, cancels)?;
                    return Ok(RouteStatus::Handled);
                }
                let ui_op = match (op, &behavior.kind) {
                    (SemanticOp::Activate, BehaviorKind::Button | BehaviorKind::Toggle) => {
                        UiOp::Activate
                    }
                    (SemanticOp::SetValue(value), BehaviorKind::Slider(range))
                        if value.is_finite() =>
                    {
                        UiOp::SetValue(range.snap(value))
                    }
                    (SemanticOp::Step(step), BehaviorKind::Slider(_)) => UiOp::Step(step),
                    (SemanticOp::Edit(edit), BehaviorKind::Text { .. }) => UiOp::Edit(edit),
                    _ => return Err(RouteError::Unsupported),
                };
                events.push(behavior.event(command, ui_op));
                if matches!(op, SemanticOp::Edit(_)) {
                    self.restart_text_session(widget, cancels)?;
                }
                Ok(RouteStatus::Handled)
            }
            CommandKind::Navigate { view, direction } if view == self.view => {
                self.navigate(command, direction, events, cancels)
            }
            CommandKind::Action {
                action,
                phase: ActionPhase::Press,
            } if self.activate_action == Some(action) => {
                if !self.valid {
                    return Ok(RouteStatus::NeedsLayout);
                }
                let Some(focus) = self.focus else {
                    return Ok(RouteStatus::Unclaimed);
                };
                let behavior = self
                    .snapshot
                    .behavior(focus.widget)
                    .ok_or(RouteError::TargetGone)?;
                if matches!(behavior.kind, BehaviorKind::Button | BehaviorKind::Toggle) {
                    events.push(behavior.event(command, UiOp::Activate));
                }
                Ok(RouteStatus::Handled)
            }
            _ => Ok(RouteStatus::Unclaimed),
        }
    }

    fn check_position(&self, at: PointerPosition) -> Result<bool, RouteError> {
        if at.view != self.view {
            return Ok(false);
        }
        if !self.initialized {
            return Ok(true);
        }
        if at.space != self.snapshot.space
            || at
                .expected_snapshot
                .is_some_and(|id| id != self.snapshot.id)
        {
            return Err(RouteError::StaleSnapshot);
        }
        Ok(true)
    }

    fn pointer<'a>(
        &mut self,
        command: CommandRef<'a>,
        pointer: PointerCommand,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
        text: Option<&TextSystem>,
    ) -> Result<RouteStatus, RouteError> {
        let (id, at, contact) = match pointer {
            PointerCommand::Cancel { id } => {
                let mut handled = false;
                self.captures.retain(|capture| {
                    if capture.source == command.source && capture.pointer == id {
                        if let Some((widget, action)) = capture.target {
                            cancels.push(UiCancel {
                                widget,
                                action,
                                source: capture.source,
                                kind: CancelKind::Capture,
                            });
                        }
                        handled = true;
                        false
                    } else {
                        true
                    }
                });
                return Ok(if handled {
                    RouteStatus::Handled
                } else {
                    RouteStatus::Unclaimed
                });
            }
            PointerCommand::Move { id, at, .. } => (id, at, None),
            PointerCommand::Down { id, at, contact } | PointerCommand::Up { id, at, contact } => {
                (id, at, Some(contact))
            }
        };
        let captured = self.captures.iter().position(|capture| {
            capture.source == command.source
                && capture.pointer == id
                && contact.is_none_or(|contact| capture.contact == contact)
        });
        // Up 必须先解除原捕获；坐标过期也不能留下持续按下状态。
        if matches!(pointer, PointerCommand::Up { .. })
            && let Some(index) = captured
        {
            let capture = self.captures.swap_remove(index);
            if let Some((widget, _)) = capture.target
                && self.check_position(at).unwrap_or(false)
                && let Some(behavior) = self.snapshot.behavior(widget)
            {
                match behavior.kind {
                    BehaviorKind::Button | BehaviorKind::Toggle
                        if self.valid && behavior.contains(point(at.point)) =>
                    {
                        events.push(behavior.event(command, UiOp::Activate))
                    }
                    BehaviorKind::Slider(_) if self.valid => {
                        if let Some(op) = behavior.point_op(point(at.point), false, text) {
                            events.push(behavior.event(command, op));
                        }
                    }
                    _ => {}
                }
            }
            return Ok(RouteStatus::Handled);
        }
        if !self.check_position(at)? {
            return Ok(RouteStatus::Unclaimed);
        }
        if let Some(index) = captured {
            let capture = self.captures[index];
            if let Some((widget, _)) = capture.target
                && let Some(behavior) = self.snapshot.behavior(widget)
                && let Some(op) = behavior.point_op(
                    point(at.point),
                    matches!(behavior.kind, BehaviorKind::Text { .. }),
                    text,
                )
            {
                events.push(behavior.event(command, op));
            }
            return Ok(RouteStatus::Handled);
        }
        if !self.valid {
            return Ok(RouteStatus::NeedsLayout);
        }
        let point = point(at.point);
        let hit = self
            .snapshot
            .behaviors
            .iter()
            .rev()
            .find(|behavior| behavior.contains(point));
        let Some(hit) = hit else {
            self.hover = None;
            return Ok(RouteStatus::Unclaimed);
        };
        self.hover = Some(hit.widget);
        if let PointerCommand::Down { contact, .. } = pointer {
            let busy = self
                .captures
                .iter()
                .any(|capture| capture.target.is_some_and(|target| target.0 == hit.widget));
            if self.captures.len() == self.limits.pointers {
                return Err(RouteError::PointerLimit);
            }
            let widget = hit.widget;
            let action = hit.action;
            let role = hit.role();
            if !hit.enabled || contact != Contact::Primary || busy {
                self.captures.push(Capture {
                    source: command.source,
                    pointer: id,
                    contact,
                    target: None,
                    role,
                });
                return Ok(RouteStatus::Handled);
            }
            let op = hit.point_op(point, false, text);
            let relocate = role == Role::TextInput
                && self
                    .focus
                    .is_some_and(|focus| focus.widget == widget && focus.source == command.source);
            let session = if relocate {
                Some(
                    self.text_session
                        .checked_add(1)
                        .ok_or(RouteError::Generation)?,
                )
            } else {
                None
            };
            self.set_focus(widget, command.source, cancels)?;
            if let Some(session) = session {
                self.text_session = session;
                cancels.push(UiCancel {
                    widget,
                    action,
                    source: command.source,
                    kind: CancelKind::TextSession,
                });
            }
            self.captures.push(Capture {
                source: command.source,
                pointer: id,
                contact,
                target: Some((widget, action)),
                role,
            });
            if let Some(op) = op {
                events.push(
                    self.snapshot
                        .behavior(widget)
                        .ok_or(RouteError::TargetGone)?
                        .event(command, op),
                );
            }
        }
        Ok(RouteStatus::Handled)
    }

    fn navigate<'a>(
        &mut self,
        command: CommandRef<'a>,
        direction: Navigation,
        events: &mut EventBuffer<'a>,
        cancels: &mut CancelBuffer,
    ) -> Result<RouteStatus, RouteError> {
        if !self.valid {
            return Ok(RouteStatus::NeedsLayout);
        }
        if let Some(focus) = self.focus
            && let Some(behavior) = self.snapshot.behavior(focus.widget)
            && let BehaviorKind::Slider(range) = behavior.kind
        {
            let op = match direction {
                Navigation::Left | Navigation::Down => Some(UiOp::Step(-1)),
                Navigation::Right | Navigation::Up => Some(UiOp::Step(1)),
                Navigation::Home => Some(UiOp::SetValue(range.min)),
                Navigation::End => Some(UiOp::SetValue(range.max)),
                _ => None,
            };
            if let Some(op) = op {
                events.push(behavior.event(command, op));
                return Ok(RouteStatus::Handled);
            }
        }
        let current = self.focus.and_then(|focus| {
            self.snapshot
                .behaviors
                .iter()
                .position(|behavior| behavior.widget == focus.widget)
        });
        let count = self.snapshot.behaviors.len();
        let backwards = matches!(
            direction,
            Navigation::Previous | Navigation::Left | Navigation::Up | Navigation::PageUp
        );
        for offset in 1..=count {
            let index = if backwards {
                (current.unwrap_or(0) + count - offset) % count
            } else {
                (current.unwrap_or(count - 1) + offset) % count
            };
            let behavior = &self.snapshot.behaviors[index];
            if behavior.enabled && !behavior.bounds.intersection(behavior.clip).is_empty() {
                self.set_focus(behavior.widget, command.source, cancels)?;
                break;
            }
        }
        Ok(RouteStatus::Handled)
    }
}

fn point(value: input::Point) -> Point {
    Point::new(value.x, value.y)
}

#[cfg(test)]
mod platform_tests {
    use super::*;
    use crate::{BuildInfo, ControlStyle, ScheduledVisual, UiStorage, Widget};
    use render2d::text::{TextBudgets, TextDocument, TextStyle, TextSystem};

    fn publish(candidate: crate::Candidate<'_, '_>) {
        let scheduled =
            ScheduledVisual::new(candidate.visual_id(), ViewId::new(1), SpaceEpoch::new(7));
        candidate.publish(scheduled, &mut Vec::new()).unwrap();
    }

    fn info() -> BuildInfo {
        BuildInfo::new(Rect::new(0.0, 0.0, 100.0, 100.0), SpaceEpoch::new(7))
    }

    #[test]
    fn semantic_visible_region_uses_the_published_space() {
        let limits = Limits::default();
        let mut ui = Ui::new(ViewId::new(1), limits).unwrap();
        let mut storage = UiStorage::new(limits).unwrap();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        let node = build
            .button(
                build.root(),
                Widget::new(WidgetId::new(1), ActionId::new(1), "button"),
                None,
                ControlStyle::default(),
            )
            .unwrap();
        build.offset(node, Point::new(90.0, 20.0)).unwrap();
        publish(build.finish().unwrap());
        let snapshot = ui.semantics().unwrap();
        assert_eq!(snapshot.space(), SpaceEpoch::new(7));
        assert_eq!(
            snapshot.iter().next().unwrap().visible_bounds,
            Rect::new(90.0, 20.0, 10.0, 32.0)
        );
    }

    #[test]
    fn semantic_slider_exposes_its_value_domain() {
        let limits = Limits::default();
        let mut ui = Ui::new(ViewId::new(1), limits).unwrap();
        let mut storage = UiStorage::new(limits).unwrap();
        let mut build = ui.begin(&mut storage, info()).unwrap();
        let domain = ValueRange::new(-2.0, 8.0, 0.25).unwrap();
        build
            .slider(
                build.root(),
                Widget::new(WidgetId::new(1), ActionId::new(1), "slider"),
                1.0,
                domain,
                ControlStyle::default(),
            )
            .unwrap();
        publish(build.finish().unwrap());
        assert_eq!(
            ui.semantics().unwrap().iter().next().unwrap().range,
            Some(domain)
        );
    }

    fn text_field(ui: &mut Ui, storage: &mut UiStorage, text: &mut TextSystem, doc: &TextDocument) {
        let mut build = ui.begin(storage, info()).unwrap();
        let node = build
            .text_edit(
                build.root(),
                Widget::new(WidgetId::new(1), ActionId::new(1), "text"),
                doc,
                ControlStyle::default(),
                TextStyle::default(),
                text,
            )
            .unwrap();
        build.offset(node, Point::new(10.0, 20.0)).unwrap();
        publish(build.finish_with_text(text).unwrap());
    }

    #[test]
    fn ime_caret_changes_when_the_new_text_layout_is_published() {
        let limits = Limits::default();
        let mut ui = Ui::new(ViewId::new(1), limits).unwrap();
        let mut storage = UiStorage::new(limits).unwrap();
        let mut text = TextSystem::new(
            [include_bytes!("../../../assets/fonts/NotoSansSC-Regular.otf").to_vec()],
            TextBudgets::default(),
        )
        .unwrap();
        let mut doc = TextDocument::new("ab").unwrap();
        text_field(&mut ui, &mut storage, &mut text, &doc);
        ui.set_focus(WidgetId::new(1), SourceId::new(1), &mut Vec::new())
            .unwrap();
        let old = ui.ime_caret(&text).unwrap();
        doc.apply(input::TextOp::Commit("新")).unwrap();
        assert_eq!(ui.ime_caret(&text), Some(old));
        text_field(&mut ui, &mut storage, &mut text, &doc);
        assert!(ui.ime_caret(&text).unwrap().x > old.x);
    }
}
