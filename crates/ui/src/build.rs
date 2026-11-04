//! 一次性描述、迭代布局和与提交对应的候选发布。

use std::{collections::HashSet, ops::Range};

use render2d::text::{TextDocument, TextLayout, TextSelection, TextStyle, TextSystem};
use render2d::{Canvas, DrawList};

use crate::route::{Behavior, BehaviorKind, InteractionSnapshot};
use crate::{
    ActionId, BuildError, CancelBuffer, Color, Limits, Point, Rect, SemanticSnapshot,
    SemanticValue, SnapshotId, SpaceEpoch, Ui, ValueRange, ViewId, WidgetId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// UI 所有者与本次构建共同确定的视觉身份；不能跨 UI 替代。
pub struct VisualId {
    owner: u64,
    sequence: u64,
}

impl VisualId {
    /// 仅该 UI 内的构建序号；跨 UI 比较应使用完整 [`VisualId`]。
    pub const fn get(self) -> u64 {
        self.sequence
    }
}

/// Host 仅在对应帧已提交并排定呈现后构造；不表示屏幕已经显示。
#[derive(Clone, Copy, Debug)]
pub struct ScheduledVisual {
    visual: VisualId,
    view: ViewId,
    space: SpaceEpoch,
}

impl ScheduledVisual {
    /// 声明 host 已提交并排定此视觉；调用者负责声明与实际提交一致。
    ///
    /// 发布时校验视觉、view 与空间代次，不能仅凭相同构建序号替换另一 UI。
    pub const fn new(visual: VisualId, view: ViewId, space: SpaceEpoch) -> Self {
        Self {
            visual,
            view,
            space,
        }
    }
}

#[derive(Debug, thiserror::Error)]
/// 候选与 host 的实际排定视觉不一致。
pub enum PublishError {
    /// 视觉所有者／序号、view 或坐标空间不匹配，旧布局保留。
    #[error("排定呈现的帧与 UI 候选不对应")]
    WrongVisual,
}

#[derive(Clone, Copy, Debug)]
/// 本次描述采用的逻辑视口与坐标空间。
pub struct BuildInfo {
    /// 根节点的逻辑像素边界，同时作为初始裁剪。
    pub bounds: Rect,
    /// 布局对应的坐标空间代次；改变映射时由 host 递增。
    pub space: SpaceEpoch,
    /// 是否允许通过语义快照发现控件；不关闭内部输入行为。
    pub semantics: bool,
}

impl BuildInfo {
    /// 使用给定逻辑视口与空间，默认启用语义发现；几何在 [`Ui::begin`] 校验。
    pub const fn new(bounds: Rect, space: SpaceEpoch) -> Self {
        Self {
            bounds,
            space,
            semantics: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// 仅属于某 UI 本次构建的节点句柄；旧构建句柄不能作为新父节点。
pub struct NodeId {
    owner: u64,
    build: u64,
    index: u32,
}

#[derive(Clone, Copy, Debug)]
/// 构建期间借用的稳定控件描述；名称同步复制，UI 不保留应用引用。
pub struct Widget<'a> {
    /// 跨更新保持不变的业务键，同次构建不能重复。
    pub id: WidgetId,
    /// 路由结果携带的业务动作；改变动作不会继承原焦点或捕获。
    pub action: ActionId,
    /// 面向语义发现的名称，不自动生成视觉文字。
    pub caption: &'a str,
}

impl<'a> Widget<'a> {
    /// 组合稳定身份、业务动作和借用名称，重复身份在加入 builder 时拒绝。
    pub const fn new(id: WidgetId, action: ActionId, caption: &'a str) -> Self {
        Self {
            id,
            action,
            caption,
        }
    }
}

#[derive(Clone, Copy, Debug)]
/// 容器的固定逻辑尺寸与流式间距；不实现 CSS 自适应尺寸。
pub struct LayoutStyle {
    /// `[宽, 高]`，有限且非负，不随子节点扩张。
    pub size: [f32; 2],
    /// 四边相同的内边距，有限且非负。
    pub padding: f32,
    /// row／column 相邻子节点之间的间距，absolute 不使用此值。
    pub gap: f32,
}

impl LayoutStyle {
    /// 创建无内边距和间隔的固定尺寸，加入构建时统一校验几何。
    pub const fn new(width: f32, height: f32) -> Self {
        Self {
            size: [width, height],
            padding: 0.0,
            gap: 0.0,
        }
    }
    fn valid(self) -> bool {
        self.size
            .into_iter()
            .all(|value| value.is_finite() && value >= 0.0)
            && self.padding.is_finite()
            && self.padding >= 0.0
            && self.gap.is_finite()
            && self.gap >= 0.0
    }
}

#[derive(Clone, Copy, Debug)]
/// 控件的固定逻辑几何与线性预乘颜色；颜色规则同 [`Color`]。
pub struct ControlStyle {
    /// `[宽, 高]`，有限且非负。
    pub size: [f32; 2],
    /// 关闭时保留绘制与语义描述，但不接受新的交互。
    pub enabled: bool,
    /// 四边相同的文字内边距，有限且非负。
    pub padding: f32,
    /// 圆角逻辑半径，有限且非负；绘制时按边界收敛。
    pub radius: f32,
    /// 控件底色。
    pub background: Color,
    /// 普通文字、边框与标记颜色。
    pub foreground: Color,
    /// 焦点、选中与活动区域颜色。
    pub accent: Color,
}

impl Default for ControlStyle {
    fn default() -> Self {
        Self {
            size: [200.0, 32.0],
            enabled: true,
            padding: 6.0,
            radius: 4.0,
            background: Color::linear(0.017, 0.024, 0.042, 1.0),
            foreground: Color::linear(0.753, 0.807, 0.913, 1.0),
            accent: Color::linear(0.061, 0.352, 0.831, 1.0),
        }
    }
}

impl ControlStyle {
    fn valid(self) -> bool {
        LayoutStyle::new(self.size[0], self.size[1]).valid()
            && self.padding.is_finite()
            && self.padding >= 0.0
            && self.radius.is_finite()
            && self.radius >= 0.0
    }
}

#[derive(Clone, Copy)]
enum Flow {
    Absolute,
    Row,
    Column,
}

enum NodeKind {
    Container {
        flow: Flow,
        padding: f32,
        gap: f32,
    },
    Label,
    Button,
    Toggle(bool),
    Slider {
        value: f64,
        range: ValueRange,
    },
    Text {
        revision: input::TextRevision,
        selection: TextSelection,
        caret: render2d::text::TextCursor,
        nowrap: bool,
    },
}

struct Node {
    parent: Option<usize>,
    offset: Point,
    size: [f32; 2],
    bounds: Rect,
    clip: Rect,
    cursor: f32,
    kind: NodeKind,
    widget: Option<(WidgetId, ActionId)>,
    style: ControlStyle,
    name: Range<usize>,
    text: Option<TextLayout>,
}

/// 可复用的连续描述、布局、绘制和候选快照缓冲。
///
/// 与 [`Ui`] 分开拥有，以在发布时交换候选与已发布快照、复用双方容量。
/// 同一次构建独占借用此存储，不为各控件分别分配回调或所有者。
pub struct UiStorage {
    nodes: Vec<Node>,
    ids: HashSet<WidgetId>,
    draw: DrawList,
    snapshot: InteractionSnapshot,
    selection: Vec<Rect>,
}

impl UiStorage {
    /// 预留节点和名称容量；这两项预算为零或节点超出 `u32` 时失败。
    /// 使用此存储的实际上限由当前 [`Ui`] 的预算执行。
    pub fn new(limits: Limits) -> Result<Self, BuildError> {
        if limits.nodes == 0 || limits.nodes > u32::MAX as usize || limits.semantic_bytes == 0 {
            return Err(BuildError::InvalidLimits);
        }
        Ok(Self {
            nodes: Vec::with_capacity(limits.nodes),
            ids: HashSet::with_capacity(limits.nodes),
            draw: DrawList::default(),
            snapshot: InteractionSnapshot::new(ViewId::new(0), limits.nodes, limits.semantic_bytes),
            selection: Vec::new(),
        })
    }
}

/// 独占借用 UI 与存储的一次描述；容器先于子节点加入，布局在 [`Self::finish`] 迭代求解。
///
/// 添加操作统一拒绝过期／非容器父节点、非法样式、重复控件键以及节点／名称超额。
/// 叶子尺寸显式给定；返回的 [`NodeId`] 只在本次构建内有效。
pub struct UiBuilder<'ui, 'frame> {
    ui: &'ui mut Ui,
    storage: &'frame mut UiStorage,
    build: u64,
    info: BuildInfo,
}

/// 已完成布局的候选绘制与命中快照，尚未改变当前发布状态。
///
/// host 使用 [`Self::draw`] 准备绘制，提交并排定呈现后调用 [`Self::publish`]。
/// 丢弃候选保留旧布局；候选借用阻止同时路由、重建或回收存储。
pub struct Candidate<'ui, 'frame> {
    ui: &'ui mut Ui,
    storage: &'frame mut UiStorage,
    build: u64,
}

impl Ui {
    /// 清空可复用描述缓冲，开始一次独占构建，容器定位延迟到 finish。
    ///
    /// 非法视口或构建代次耗尽时失败；不会改写已发布布局，也不重跑应用描述。
    pub fn begin<'ui, 'frame>(
        &'ui mut self,
        storage: &'frame mut UiStorage,
        info: BuildInfo,
    ) -> Result<UiBuilder<'ui, 'frame>, BuildError> {
        if !info.bounds.valid() {
            return Err(BuildError::InvalidLayout);
        }
        let build = self.next_build;
        self.next_build = build.checked_add(1).ok_or(BuildError::Generation)?;
        storage.nodes.clear();
        storage.ids.clear();
        storage.draw.clear();
        storage.snapshot.clear();
        storage.snapshot.id = SnapshotId::new(build);
        storage.snapshot.view = self.view;
        storage.snapshot.space = info.space;
        storage.snapshot.discover = info.semantics;
        storage.nodes.push(Node {
            parent: None,
            offset: Point::new(0.0, 0.0),
            size: [info.bounds.width, info.bounds.height],
            bounds: info.bounds,
            clip: info.bounds,
            cursor: 0.0,
            kind: NodeKind::Container {
                flow: Flow::Absolute,
                padding: 0.0,
                gap: 0.0,
            },
            widget: None,
            style: ControlStyle::default(),
            name: 0..0,
            text: None,
        });
        Ok(UiBuilder {
            ui: self,
            storage,
            build,
            info,
        })
    }
}

impl<'ui, 'frame> UiBuilder<'ui, 'frame> {
    /// 覆盖本次 [`BuildInfo::bounds`] 的 absolute 根节点，可包含子节点。
    pub const fn root(&self) -> NodeId {
        NodeId {
            owner: self.ui.owner,
            build: self.build,
            index: 0,
        }
    }

    fn container(
        &mut self,
        parent: NodeId,
        flow: Flow,
        style: LayoutStyle,
    ) -> Result<NodeId, BuildError> {
        if !style.valid() {
            return Err(BuildError::InvalidLayout);
        }
        self.push(
            parent,
            NodeKind::Container {
                flow,
                padding: style.padding,
                gap: style.gap,
            },
            None,
            "",
            None,
            ControlStyle {
                size: style.size,
                ..ControlStyle::default()
            },
        )
    }

    /// 加入横向依次定位子节点的固定尺寸容器；错误遵循 [`UiBuilder`] 的统一边界。
    pub fn row(&mut self, parent: NodeId, style: LayoutStyle) -> Result<NodeId, BuildError> {
        self.container(parent, Flow::Row, style)
    }
    /// 加入纵向依次定位子节点的固定尺寸容器；错误遵循 [`UiBuilder`] 的统一边界。
    pub fn column(&mut self, parent: NodeId, style: LayoutStyle) -> Result<NodeId, BuildError> {
        self.container(parent, Flow::Column, style)
    }
    /// 加入子节点共用内容起点的容器，通过 [`Self::offset`] 分别定位。
    /// 错误遵循 [`UiBuilder`] 的统一边界。
    pub fn absolute(&mut self, parent: NodeId, style: LayoutStyle) -> Result<NodeId, BuildError> {
        self.container(parent, Flow::Absolute, style)
    }

    /// 只改变本次节点相对位置，不改变父子关系。
    /// 根节点、其他构建节点或非有限偏移返回 [`BuildError::InvalidParent`]。
    pub fn offset(&mut self, node: NodeId, offset: Point) -> Result<(), BuildError> {
        if node.owner != self.ui.owner
            || node.build != self.build
            || node.index == 0
            || !offset.x.is_finite()
            || !offset.y.is_finite()
        {
            return Err(BuildError::InvalidParent);
        }
        self.storage
            .nodes
            .get_mut(node.index as usize)
            .ok_or(BuildError::InvalidParent)?
            .offset = offset;
        Ok(())
    }

    /// 加入不可交互文字，尺寸取已准备的布局；不创建语义控件。
    /// 错误遵循 [`UiBuilder`] 的统一边界，布局所有权转入候选。
    pub fn label(
        &mut self,
        parent: NodeId,
        layout: TextLayout,
        color: Color,
    ) -> Result<NodeId, BuildError> {
        let size = layout.size();
        self.push(
            parent,
            NodeKind::Label,
            None,
            "",
            Some(layout),
            ControlStyle {
                size,
                foreground: color,
                padding: 0.0,
                ..ControlStyle::default()
            },
        )
    }

    /// 加入按钮；主接触在原控件内释放或逻辑激活时产生 [`crate::UiOp::Activate`]。
    ///
    /// caption 用于语义发现；视觉文字由 layout 提供，图标按钮允许 `None`。
    /// 错误遵循 [`UiBuilder`] 的统一边界，UI 不替应用执行动作。
    pub fn button(
        &mut self,
        parent: NodeId,
        widget: Widget<'_>,
        layout: Option<TextLayout>,
        style: ControlStyle,
    ) -> Result<NodeId, BuildError> {
        self.push(
            parent,
            NodeKind::Button,
            Some((widget.id, widget.action)),
            widget.caption,
            layout,
            style,
        )
    }
    /// 加入由应用提供选中状态的开关，交互只发出激活事件，不自行翻转业务值。
    /// 错误遵循 [`UiBuilder`] 的统一边界。
    pub fn toggle(
        &mut self,
        parent: NodeId,
        widget: Widget<'_>,
        checked: bool,
        layout: Option<TextLayout>,
        style: ControlStyle,
    ) -> Result<NodeId, BuildError> {
        self.push(
            parent,
            NodeKind::Toggle(checked),
            Some((widget.id, widget.action)),
            widget.caption,
            layout,
            style,
        )
    }
    /// 加入量化数值拖动条；指针值裁剪并吸附到范围，键盘／语义步进交给应用执行。
    /// 当前值非有限或超出范围时返回 [`BuildError::InvalidValue`]，其他错误同 [`UiBuilder`]。
    pub fn slider(
        &mut self,
        parent: NodeId,
        widget: Widget<'_>,
        value: f64,
        range: ValueRange,
        style: ControlStyle,
    ) -> Result<NodeId, BuildError> {
        if !value.is_finite() || value < range.min() || value > range.max() {
            return Err(BuildError::InvalidValue);
        }
        self.push(
            parent,
            NodeKind::Slider { value, range },
            Some((widget.id, widget.action)),
            widget.caption,
            None,
            style,
        )
    }
    /// 按文档当前修订准备文本输入，独立持有不可变布局，不借用正文到本调用之后。
    ///
    /// `font.wrap = false` 时横向滚动以跟随 caret，绘制、命中和 IME 共享滚动坐标。
    /// 编辑结果仍交给应用的文档处理；文字预算／布局失败保留 [`BuildError::Text`] 来源，
    /// 其他错误遵循 [`UiBuilder`] 的统一边界。
    pub fn text_edit(
        &mut self,
        parent: NodeId,
        widget: Widget<'_>,
        document: &TextDocument,
        style: ControlStyle,
        font: TextStyle,
        text: &mut TextSystem,
    ) -> Result<NodeId, BuildError> {
        if !style.valid() {
            return Err(BuildError::InvalidLayout);
        }
        let layout = document.layout(text, (style.size[0] - 2.0 * style.padding).max(1.0), font)?;
        self.push(
            parent,
            NodeKind::Text {
                revision: document.revision(),
                selection: document.selection(),
                caret: document.caret(),
                nowrap: !font.wrap,
            },
            Some((widget.id, widget.action)),
            widget.caption,
            Some(layout),
            style,
        )
    }

    fn push(
        &mut self,
        parent: NodeId,
        kind: NodeKind,
        widget: Option<(WidgetId, ActionId)>,
        caption: &str,
        text: Option<TextLayout>,
        style: ControlStyle,
    ) -> Result<NodeId, BuildError> {
        let parent_index = parent.index as usize;
        if parent.owner != self.ui.owner
            || parent.build != self.build
            || !self
                .storage
                .nodes
                .get(parent_index)
                .is_some_and(|node| matches!(node.kind, NodeKind::Container { .. }))
        {
            return Err(BuildError::InvalidParent);
        }
        if !style.valid() {
            return Err(BuildError::InvalidLayout);
        }
        if self.storage.nodes.len() == self.ui.limits.nodes {
            return Err(BuildError::Capacity);
        }
        if let Some((id, _)) = widget
            && self.storage.ids.contains(&id)
        {
            return Err(BuildError::DuplicateWidget(id));
        }
        let names = &mut self.storage.snapshot.names;
        let start = names.len();
        if self.info.semantics && widget.is_some() {
            if caption.len() > self.ui.limits.semantic_bytes - start {
                return Err(BuildError::Capacity);
            }
            names.push_str(caption);
        }
        let index = self.storage.nodes.len() as u32;
        if let Some((id, _)) = widget {
            self.storage.ids.insert(id);
        }
        self.storage.nodes.push(Node {
            parent: Some(parent_index),
            offset: Point::default(),
            size: style.size,
            bounds: Rect::default(),
            clip: Rect::default(),
            cursor: 0.0,
            kind,
            widget,
            style,
            name: start..names.len(),
            text,
        });
        Ok(NodeId {
            owner: self.ui.owner,
            build: self.build,
            index,
        })
    }

    /// 顺序计算一次布局与绘制，返回尚未发布的候选，不重跑业务描述。
    ///
    /// 父节点先于子节点，布局和绘制均迭代扫描；裁剪统一用于绘制和命中。
    /// 几何溢出、文字或场景非法时失败；当前已发布布局保持不变。
    pub fn finish(self) -> Result<Candidate<'ui, 'frame>, BuildError> {
        self.finish_impl(None)
    }
    /// 借用文字所有者解析本次候选；身份过期时保留旧发布状态并失败。
    pub fn finish_with_text(self, text: &TextSystem) -> Result<Candidate<'ui, 'frame>, BuildError> {
        self.finish_impl(Some(text))
    }
    fn finish_impl(self, text: Option<&TextSystem>) -> Result<Candidate<'ui, 'frame>, BuildError> {
        for index in 1..self.storage.nodes.len() {
            let parent = self.storage.nodes[index]
                .parent
                .ok_or(BuildError::InvalidParent)?;
            let (before, after) = self.storage.nodes.split_at_mut(index);
            let parent_node = &mut before[parent];
            let node = &mut after[0];
            let NodeKind::Container { flow, padding, gap } = parent_node.kind else {
                return Err(BuildError::InvalidParent);
            };
            let content = parent_node.bounds.inset(padding);
            let (x, y) = match flow {
                Flow::Absolute => (content.x, content.y),
                Flow::Row => {
                    let x = content.x + parent_node.cursor;
                    parent_node.cursor += node.size[0] + gap;
                    (x, content.y)
                }
                Flow::Column => {
                    let y = content.y + parent_node.cursor;
                    parent_node.cursor += node.size[1] + gap;
                    (content.x, y)
                }
            };
            node.bounds = Rect::new(
                x + node.offset.x,
                y + node.offset.y,
                node.size[0],
                node.size[1],
            );
            node.clip = parent_node
                .clip
                .intersection(content)
                .intersection(node.bounds);
            if !node.bounds.valid() {
                return Err(BuildError::InvalidLayout);
            }
        }
        let UiStorage {
            nodes,
            draw,
            snapshot,
            selection,
            ..
        } = &mut *self.storage;
        for node in nodes {
            let mut canvas = draw.canvas(node.clip);
            let style = node.style;
            let mut text_origin =
                Point::new(node.bounds.x + style.padding, node.bounds.y + style.padding);
            let viewport = node.bounds.inset(style.padding);
            let mut scroll_x = 0.0;
            if let NodeKind::Text {
                nowrap: true,
                caret,
                ..
            } = node.kind
                && let Some(layout) = node
                    .text
                    .as_ref()
                    .and_then(|layout| text.and_then(|owner| owner.resolve(layout)))
                && viewport.width > 0.0
            {
                if let Some((widget, action)) = node.widget
                    && let Some(old) = self
                        .ui
                        .snapshot
                        .indices
                        .get(&widget)
                        .and_then(|index| self.ui.snapshot.behaviors.get(*index))
                        .filter(|old| old.action == action)
                    && let BehaviorKind::Text { scroll_x: old, .. } = old.kind
                {
                    scroll_x = old;
                }
                let [left, right] = layout.advance_x();
                let min = left.min(0.0);
                let max = (right + 1.0 - viewport.width).max(0.0);
                scroll_x = scroll_x.clamp(min, max);
                if let Some(caret) = layout.caret_rect(caret) {
                    if caret.x < scroll_x {
                        scroll_x = caret.x;
                    } else if caret.right() > scroll_x + viewport.width {
                        scroll_x = caret.right() - viewport.width;
                    }
                    scroll_x = scroll_x.clamp(min, max);
                }
                text_origin.x -= scroll_x;
            }
            let opacity = if style.enabled { 1.0 } else { 0.45 };
            let mut background = style.background.opacity(opacity);
            let foreground = style.foreground.opacity(opacity);
            let accent = style.accent.opacity(opacity);
            if let Some((widget, _)) = node.widget
                && self.ui.pressed(widget)
            {
                background = background.opacity(0.7);
            }
            match node.kind {
                NodeKind::Button | NodeKind::Toggle(_) | NodeKind::Text { .. } => {
                    canvas.rounded_rect(node.bounds, style.radius, background)
                }
                NodeKind::Slider { value, range } => {
                    let track = Rect::new(
                        node.bounds.x,
                        node.bounds.y + node.bounds.height * 0.4,
                        node.bounds.width,
                        node.bounds.height * 0.2,
                    );
                    canvas.rounded_rect(track, style.radius, background);
                    let fraction = range.fraction(value);
                    canvas.rounded_rect(
                        Rect::new(track.x, track.y, track.width * fraction, track.height),
                        style.radius,
                        accent,
                    );
                    canvas.rounded_rect(
                        Rect::new(
                            (node.bounds.x + node.bounds.width * fraction - 4.0).clamp(
                                node.bounds.x,
                                node.bounds.right() - 8.0_f32.min(node.bounds.width),
                            ),
                            node.bounds.y,
                            8.0_f32.min(node.bounds.width),
                            node.bounds.height,
                        ),
                        3.0,
                        foreground,
                    );
                }
                _ => {}
            }
            if let NodeKind::Toggle(checked) = node.kind {
                let side = (node.bounds.height - 2.0 * style.padding).max(0.0);
                canvas.rounded_rect(
                    Rect::new(
                        node.bounds.right() - side - style.padding,
                        text_origin.y,
                        side,
                        side,
                    ),
                    style.radius,
                    if checked {
                        accent
                    } else {
                        foreground.opacity(0.2)
                    },
                );
            }
            if let Some((widget, _)) = node.widget {
                if self.ui.hovered() == Some(widget) {
                    canvas.rounded_rect(node.bounds, style.radius, accent.opacity(0.15));
                }
                if self.ui.focus() == Some(widget) {
                    canvas.border(node.bounds, style.radius, 1.0, accent);
                }
            }
            if let Some(identity) = &node.text {
                let layout = text
                    .and_then(|owner| owner.resolve(identity))
                    .ok_or(BuildError::InvalidLayout)?;
                let text_clip = if matches!(node.kind, NodeKind::Text { .. }) {
                    viewport
                } else {
                    node.clip
                };
                let mut canvas = canvas.clipped(text_clip);
                if let NodeKind::Text {
                    selection: selected,
                    caret,
                    ..
                } = node.kind
                    && self
                        .ui
                        .focus()
                        .is_some_and(|focus| node.widget.is_some_and(|widget| widget.0 == focus))
                {
                    selection.clear();
                    layout.selection_rects_into(selected, selection);
                    for rect in selection.iter() {
                        canvas.fill_rect(
                            Rect::new(
                                rect.x + text_origin.x,
                                rect.y + text_origin.y,
                                rect.width,
                                rect.height,
                            ),
                            accent.opacity(0.35),
                        );
                    }
                    if let Some(rect) = layout.caret_rect(caret) {
                        canvas.fill_rect(
                            Rect::new(
                                rect.x + text_origin.x,
                                rect.y + text_origin.y,
                                rect.width.max(1.0),
                                rect.height,
                            ),
                            foreground,
                        );
                    }
                    selection.clear();
                    layout.composition_rects_into(selection);
                    for rect in selection.iter() {
                        canvas.fill_rect(
                            Rect::new(
                                rect.x + text_origin.x,
                                rect.bottom() + text_origin.y - 1.0,
                                rect.width,
                                1.0,
                            ),
                            accent,
                        );
                    }
                }
                canvas.text(&layout, text_origin, foreground);
            }
            let Some((widget, action)) = node.widget else {
                continue;
            };
            let (kind, value) = match node.kind {
                NodeKind::Button => (BehaviorKind::Button, SemanticValue::None),
                NodeKind::Toggle(value) => (BehaviorKind::Toggle, SemanticValue::Toggle(value)),
                NodeKind::Slider { value, range } => {
                    (BehaviorKind::Slider(range), SemanticValue::Number(value))
                }
                NodeKind::Text {
                    revision, caret, ..
                } => (
                    BehaviorKind::Text {
                        revision,
                        layout: node.text.take().ok_or(BuildError::InvalidLayout)?,
                        origin: text_origin,
                        caret,
                        scroll_x,
                        viewport: viewport.intersection(node.clip),
                    },
                    SemanticValue::TextRevision(revision),
                ),
                _ => continue,
            };
            snapshot.indices.insert(widget, snapshot.behaviors.len());
            snapshot.behaviors.push(Behavior {
                widget,
                action,
                bounds: node.bounds,
                clip: node.clip,
                enabled: style.enabled,
                name: node.name.clone(),
                value,
                kind,
            });
        }
        self.storage.draw.validate()?;
        Ok(Candidate {
            ui: self.ui,
            storage: self.storage,
            build: self.build,
        })
    }
}

impl Candidate<'_, '_> {
    /// 当前发布与本候选输入布局的保留集合，不复制正文或字形。
    ///
    /// 候选跨越 TextSystem 代次时，须将此集合传给 `begin_epoch`，直至发布或取消。
    /// 普通标签已展开为 glyph 请求，不再访问布局负载；发布后改用 [`Ui::text_layouts`]。
    pub fn text_layouts(&self) -> impl Iterator<Item = &TextLayout> {
        self.ui
            .text_layouts()
            .chain(self.storage.snapshot.behaviors.iter().filter_map(
                |behavior| match &behavior.kind {
                    BehaviorKind::Text { layout, .. } => Some(layout),
                    _ => None,
                },
            ))
    }

    /// 最终有序绘制场景，借用到候选被发布或丢弃前。
    pub fn draw(&self) -> &DrawList {
        &self.storage.draw
    }
    /// 在 UI 绘制之后追加自定义图形，以根边界裁剪；不添加命中或语义节点。
    pub fn canvas(&mut self) -> Canvas<'_> {
        self.storage.draw.canvas(self.storage.nodes[0].bounds)
    }
    /// host 提交与发布必须使用的完整视觉身份。
    pub const fn visual_id(&self) -> VisualId {
        VisualId {
            owner: self.ui.owner,
            sequence: self.build,
        }
    }
    /// 候选的只读语义预览；尚不能作为当前屏幕的路由依据，关闭发现时为 `None`。
    pub fn semantics(&self) -> Option<SemanticSnapshot<'_>> {
        self.storage.snapshot.discover.then_some(SemanticSnapshot {
            snapshot: &self.storage.snapshot,
        })
    }
    /// 本候选节点的逻辑边界；其他 UI、构建或无效索引返回 `None`。
    pub fn bounds(&self, node: NodeId) -> Option<Rect> {
        (node.owner == self.ui.owner && node.build == self.build)
            .then(|| {
                self.storage
                    .nodes
                    .get(node.index as usize)
                    .map(|node| node.bounds)
            })
            .flatten()
    }
    /// 发布与 host 排定视觉完全匹配的命中快照，并追加失效 owner 的取消通知。
    ///
    /// 空间改变时取消整个 view；消失、禁用或改变动作／角色的控件不继承旧焦点与捕获。
    /// 不匹配返回 [`PublishError::WrongVisual`] 并保留旧布局；成功也不表示物理屏幕已显示。
    pub fn publish(
        self,
        scheduled: ScheduledVisual,
        cancels: &mut CancelBuffer,
    ) -> Result<(), PublishError> {
        if scheduled.visual != self.visual_id()
            || scheduled.view != self.ui.view
            || scheduled.space != self.storage.snapshot.space
        {
            return Err(PublishError::WrongVisual);
        }
        if self.ui.initialized && self.ui.snapshot.space != self.storage.snapshot.space {
            self.ui
                .invalidate_interaction(crate::CancelScope::View, cancels);
        }
        std::mem::swap(&mut self.ui.snapshot, &mut self.storage.snapshot);
        self.ui.valid = true;
        self.ui.initialized = true;
        self.ui.reconcile(cancels);
        Ok(())
    }
}
