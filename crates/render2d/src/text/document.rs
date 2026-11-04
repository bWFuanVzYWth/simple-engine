//! 正文、选区、IME 与修订的独占所有者；布局只接收显示文本及其位置映射。

use std::ops::Range;

use input::{EditUnit, TextMovement, TextOp, TextOpRef, TextRange, TextRevision};
use unicode_segmentation::UnicodeSegmentation;

use super::{CompositionMap, TextBudgets, TextError, TextLayout, TextStyle, TextSystem};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// 正文中的 UTF-8 字节位置；自身不绑定文档或版本。
pub struct TextCursor {
    /// 字节偏移，允许位于正文末尾。
    pub byte: usize,
}

impl TextCursor {
    /// 保存偏移，不校验文档范围或字素边界。
    pub const fn new(byte: usize) -> Self {
        Self { byte }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// 有方向的正文选区，以 UTF-8 字节计；文档维护其 grapheme 边界。
pub struct TextSelection {
    /// 扩展选区时保持不动的一端。
    pub anchor: usize,
    /// 当前 caret 所在端。
    pub focus: usize,
}

impl TextSelection {
    /// 两端相同，无选中文字。
    pub const fn is_empty(self) -> bool {
        self.anchor == self.focus
    }

    /// 忽略方向的半开字节范围。
    pub fn range(self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
/// 单次编辑实际改变的状态；可据此决定模型或视觉更新。
pub struct TextChange {
    /// 已提交正文改变。
    pub text: bool,
    /// 正文选区改变。
    pub selection: bool,
    /// 组合串、其选区或组合状态改变。
    pub composition: bool,
}

impl TextChange {
    /// 任一状态改变，包含只改变选区或组合显示的情况。
    pub const fn changed(self) -> bool {
        self.text || self.selection || self.composition
    }
}

#[derive(Clone, Copy, Debug)]
/// 借用的 IME 组合显示；正文尚未提交。
pub struct Composition<'a> {
    /// 将替换原选区的组合串。
    pub text: &'a str,
    /// 相对组合串的 UTF-8 字节范围；None 表示平台未给出 caret。
    pub selection: Option<TextRange>,
}

#[derive(Clone, Debug)]
struct Composing {
    text: String,
    display: String,
    selection: Option<TextRange>,
    replace: Range<usize>,
}

/// 正文、选区与组合显示的唯一可变所有者；布局只持有不可变副本。
///
/// 新文档光标位于开头，初始 revision 为 1。组合输入记住开始时的替换范围，
/// 结束组合不修改正文；移动、删除和命中会结束组合。平台须先过滤过期 IME 会话。
pub struct TextDocument {
    text: String,
    selection: TextSelection,
    composing: Option<Composing>,
    revision: TextRevision,
    max_bytes: usize,
}

impl TextDocument {
    /// 使用默认 [`TextBudgets::max_text_bytes`] 上限，超限返回 [`TextError::Budget`]。
    pub fn new(text: &str) -> Result<Self, TextError> {
        Self::with_limit(text, TextBudgets::default().max_text_bytes)
    }

    /// max_bytes 同时约束正文和组合中的显示文本，不预分配整个上限。
    pub fn with_limit(text: &str, max_bytes: usize) -> Result<Self, TextError> {
        if text.len() > max_bytes {
            return Err(TextError::Budget);
        }
        Ok(Self {
            text: text.to_owned(),
            selection: TextSelection::default(),
            composing: None,
            revision: TextRevision::new(1),
            max_bytes,
        })
    }

    /// 已提交正文，不包含正在预览的 IME 组合串。
    pub fn text(&self) -> &str {
        &self.text
    }
    /// 正文／组合显示的版本；仅改变选区不递增。
    pub const fn revision(&self) -> TextRevision {
        self.revision
    }
    /// 正文中的有向选区，不以组合显示字节编号。
    pub const fn selection(&self) -> TextSelection {
        self.selection
    }
    /// 正文选区的 focus，组合期间可用布局映射到 IME caret。
    pub const fn caret(&self) -> TextCursor {
        TextCursor::new(self.selection.focus)
    }

    /// 当前组合串及其局部选区；无活动组合时为 None。
    pub fn composition(&self) -> Option<Composition<'_>> {
        self.composing.as_ref().map(|value| Composition {
            text: &value.text,
            selection: value.selection,
        })
    }

    /// 顺序应用一条编辑操作，返回实际变化；拒绝超预算、无效 IME 选区和版本溢出。
    ///
    /// grapheme／word 移动依据 Unicode 边界，LineStart／LineEnd 使用正文换行，
    /// 不按视觉折行移动。Commit 替换当前选区或组合开始时记住的范围。
    pub fn apply(&mut self, op: TextOpRef<'_>) -> Result<TextChange, TextError> {
        match op {
            TextOp::Commit(text) => self.commit(text),
            TextOp::Preedit { text, selection } => self.preedit(text, selection),
            TextOp::EndComposition => self.end_composition(),
            TextOp::Delete(unit) => self.delete(unit),
            TextOp::MoveSelection { movement, extend } => self.move_selection(movement, extend),
        }
    }

    /// 应用布局命中，先核对 revision 和 UTF-8 边界，再向后对齐到 grapheme 边界。
    ///
    /// 成功会结束组合；`extend` 保留原 anchor。过期命中返回
    /// [`TextError::StaleRevision`]，不改变文档。
    pub fn hit(
        &mut self,
        revision: TextRevision,
        cursor: TextCursor,
        extend: bool,
    ) -> Result<TextChange, TextError> {
        if revision != self.revision {
            return Err(TextError::StaleRevision);
        }
        if !self.text.is_char_boundary(cursor.byte) {
            return Err(TextError::InvalidCursor);
        }
        // 组合串可改变相邻字素分组，显示命中回到正文后重新对齐。
        let focus = next_boundary(&self.text, cursor.byte);
        let mut change = self.end_composition()?;
        let old = self.selection;
        self.selection.focus = focus;
        if !extend {
            self.selection.anchor = focus;
        }
        change.selection = self.selection != old;
        Ok(change)
    }

    /// 排版正文或组合显示，并附加本文档 revision 与显示／正文位置映射。
    /// width 以逻辑像素计，参数和缓存限制同 [`TextSystem::layout`]。
    pub fn layout(
        &self,
        system: &mut TextSystem,
        width: f32,
        style: TextStyle,
    ) -> Result<TextLayout, TextError> {
        let mut layout = if let Some(value) = &self.composing {
            let mut layout = system.layout(&value.display, width, style)?;
            layout.composition = Some(CompositionMap {
                replace: value.replace.clone(),
                inserted: value.text.len(),
                caret: value
                    .selection
                    .map(|selection| value.replace.start + selection.end()),
                document_caret: self.selection.focus,
            });
            layout
        } else {
            system.layout(&self.text, width, style)?
        };
        layout.revision = Some(self.revision);
        Ok(layout)
    }

    fn next_revision(&self) -> Result<TextRevision, TextError> {
        self.revision
            .get()
            .checked_add(1)
            .map(TextRevision::new)
            .ok_or(TextError::RevisionOverflow)
    }

    fn check_replacement(&self, range: &Range<usize>, text: &str) -> Result<(), TextError> {
        if self
            .text
            .len()
            .checked_sub(range.len())
            .and_then(|len| len.checked_add(text.len()))
            .is_none_or(|len| len > self.max_bytes)
        {
            return Err(TextError::Budget);
        }
        Ok(())
    }

    fn commit(&mut self, text: &str) -> Result<TextChange, TextError> {
        let range = self
            .composing
            .as_ref()
            .map_or_else(|| self.selection.range(), |value| value.replace.clone());
        self.check_replacement(&range, text)?;
        let text_changed = &self.text[range.clone()] != text;
        let composition = self.composing.is_some();
        let revision = if text_changed || composition {
            self.next_revision()?
        } else {
            self.revision
        };
        let old = self.selection;
        self.text.replace_range(range.clone(), text);
        let focus = next_boundary(&self.text, range.start + text.len());
        self.selection = TextSelection {
            anchor: focus,
            focus,
        };
        self.composing = None;
        self.revision = revision;
        Ok(TextChange {
            text: text_changed,
            selection: self.selection != old,
            composition,
        })
    }

    fn preedit(
        &mut self,
        text: &str,
        selection: Option<TextRange>,
    ) -> Result<TextChange, TextError> {
        if selection.is_some_and(|range| text.get(range.start()..range.end()).is_none()) {
            return Err(TextError::InvalidComposition);
        }
        if text.is_empty() {
            return self.end_composition();
        }
        let range = self
            .composing
            .as_ref()
            .map_or_else(|| self.selection.range(), |value| value.replace.clone());
        self.check_replacement(&range, text)?;
        if self
            .composing
            .as_ref()
            .is_some_and(|value| value.text == text && value.selection == selection)
        {
            return Ok(TextChange::default());
        }
        let revision = self.next_revision()?;
        if let Some(value) = &mut self.composing {
            value.text.clear();
            value.text.push_str(text);
            value.selection = selection;
            value.display.clear();
            value.display.push_str(&self.text[..range.start]);
            value.display.push_str(text);
            value.display.push_str(&self.text[range.end..]);
        } else {
            let mut display = String::with_capacity(self.text.len() - range.len() + text.len());
            display.push_str(&self.text[..range.start]);
            display.push_str(text);
            display.push_str(&self.text[range.end..]);
            self.composing = Some(Composing {
                text: text.to_owned(),
                display,
                selection,
                replace: range,
            });
        }
        self.revision = revision;
        Ok(TextChange {
            composition: true,
            ..Default::default()
        })
    }

    fn end_composition(&mut self) -> Result<TextChange, TextError> {
        if self.composing.is_none() {
            return Ok(TextChange::default());
        }
        let revision = self.next_revision()?;
        self.composing = None;
        self.revision = revision;
        Ok(TextChange {
            composition: true,
            ..Default::default()
        })
    }

    fn delete(&mut self, unit: EditUnit) -> Result<TextChange, TextError> {
        let mut range = self.selection.range();
        if range.is_empty() {
            let at = range.start;
            range = match unit {
                EditUnit::GraphemeBackward => previous_grapheme(&self.text, at)..at,
                EditUnit::GraphemeForward => at..next_grapheme(&self.text, at),
                EditUnit::WordBackward => previous_word(&self.text, at)..at,
                EditUnit::WordForward => at..next_word(&self.text, at),
            };
        }
        let changed = !range.is_empty();
        let composition = self.composing.is_some();
        let revision = if changed || composition {
            self.next_revision()?
        } else {
            self.revision
        };
        let old = self.selection;
        self.text.replace_range(range.clone(), "");
        let focus = next_boundary(&self.text, range.start);
        self.selection = TextSelection {
            anchor: focus,
            focus,
        };
        self.composing = None;
        self.revision = revision;
        Ok(TextChange {
            text: changed,
            selection: self.selection != old,
            composition,
        })
    }

    fn move_selection(
        &mut self,
        movement: TextMovement,
        extend: bool,
    ) -> Result<TextChange, TextError> {
        let at = self.selection.focus;
        let backwards = matches!(
            movement,
            TextMovement::GraphemeBackward | TextMovement::WordBackward
        );
        let focus = if !extend
            && !self.selection.is_empty()
            && matches!(
                movement,
                TextMovement::GraphemeBackward | TextMovement::GraphemeForward
            ) {
            let range = self.selection.range();
            if backwards { range.start } else { range.end }
        } else {
            match movement {
                TextMovement::GraphemeBackward => previous_grapheme(&self.text, at),
                TextMovement::GraphemeForward => next_grapheme(&self.text, at),
                TextMovement::WordBackward => previous_word(&self.text, at),
                TextMovement::WordForward => next_word(&self.text, at),
                TextMovement::LineStart => self.text[..at]
                    .rfind(['\n', '\r'])
                    .map_or(0, |index| index + 1),
                TextMovement::LineEnd => self.text[at..]
                    .find(['\n', '\r'])
                    .map_or(self.text.len(), |index| at + index),
                TextMovement::DocumentStart => 0,
                TextMovement::DocumentEnd => self.text.len(),
            }
        };
        let mut change = self.end_composition()?;
        let old = self.selection;
        self.selection.focus = focus;
        if !extend {
            self.selection.anchor = focus;
        }
        change.selection = self.selection != old;
        Ok(change)
    }
}

fn next_boundary(text: &str, byte: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .find(|index| *index >= byte)
        .unwrap_or(text.len())
}

fn previous_grapheme(text: &str, at: usize) -> usize {
    text[..at]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_grapheme(text: &str, at: usize) -> usize {
    text[at..]
        .graphemes(true)
        .next()
        .map_or(at, |part| at + part.len())
}

// UAX #29；向后停在词首，向前停在词尾。
// https://www.unicode.org/reports/tr29/
fn previous_word(text: &str, at: usize) -> usize {
    text[..at]
        .unicode_word_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_word(text: &str, at: usize) -> usize {
    text[at..]
        .unicode_word_indices()
        .next()
        .map_or(text.len(), |(index, word)| at + index + word.len())
}
