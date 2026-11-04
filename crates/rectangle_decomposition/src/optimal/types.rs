//! 矩形分解的公共类型。

#[cfg(verus_only)]
use vstd::{prelude::*, std_specs::iter::IteratorSpec};

#[path = "../proof/chords.rs"]
#[cfg(verus_only)]
mod chords;
#[path = "../proof/packed.rs"]
#[cfg(verus_only)]
mod proof;

/// 左闭右开的 `u8` 范围，模仿 `core::ops::Range` 命名。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct RangeU8 {
    /// 起始坐标（含）。
    pub start: u8,
    /// 结束坐标（不含）。
    pub end: u8,
}

#[cfg_attr(verus_only, verus_verify)]
impl RangeU8 {
    /// 原样保存边界，不检查起止顺序；分解输出保证非空且终点不超过 64。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result.start == start, result.end == end))]
    pub const fn new(start: u8, end: u8) -> Self {
        Self { start, end }
    }
}

impl From<RangeU8> for core::ops::Range<u8> {
    fn from(r: RangeU8) -> Self {
        r.start..r.end
    }
}

/// 分解结果矩形。
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[cfg_attr(verus_only, verus_verify)]
pub struct Rectangle {
    /// 矩形覆盖的像素值。
    pub value: u16,
    /// x 范围，左闭右开。
    pub x: RangeU8,
    /// y 范围，左闭右开。
    pub y: RangeU8,
}

#[cfg_attr(verus_only, verus_verify)]
impl Rectangle {
    /// 按低到高字节编码四个边界，不依赖宿主字节序。
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::encoded(self)))]
    pub(crate) const fn packed_bounds(self) -> u32 {
        (self.x.start as u32)
            | ((self.x.end as u32) << 8)
            | ((self.y.start as u32) << 16)
            | ((self.y.end as u32) << 24)
    }

    /// 解码数值字的四个字节，完整保留标签；不检查矩形合法性。
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == proof::decoded(bounds, value)))]
    pub(crate) const fn from_packed(bounds: u32, value: u16) -> Self {
        Self {
            value,
            x: RangeU8::new(bounds as u8, (bounds >> 8) as u8),
            y: RangeU8::new((bounds >> 16) as u8, (bounds >> 24) as u8),
        }
    }
}

/// 调用方持有的固定容量矩形 `SoA` 输出，按 64 字节对齐。
///
/// bounds 和 labels 数组的起点均按 64 字节对齐。
/// 每个矩形占 4 字节 bounds 和 2 字节 label；bounds 的低到高字节依次为
/// `x.start, x.end, y.start, y.end`。坐标采用左闭右开范围，终点可为 64。
/// 所有存储在初始化时置零，清空只修改长度，不分配或重新初始化数组。
///
/// 验证目标：始终有 `len <= 4096`，`bounds().len() = labels().len() = len`；
/// 第 i 项解码为本次分解对应的成功写入矩形，编码与解码互逆，不受宿主字节序影响。
/// `iter()` 按有效索引恰好产出 len 项，不读取尾部旧存储；`clear()` 后视图和迭代均为空。
/// packed 分解先清空旧输出，成功时返回值等于 len；失败时仅保留本次前缀。
/// 矩形合法性由分解生产者建立，公开的 [`Rectangle`]／[`RangeU8`] 字段不保证合法性。
/// 同源证明覆盖本类型的固有方法、Default 和实际借用迭代器；分解生产者仍须单独验证。
/// 私有长度及类型不变量维持访问边界，安全公开调用无需额外校验。
#[repr(C, align(64))]
#[cfg_attr(verus_only, verus_verify)]
pub struct PackedRectangles64 {
    bounds: [u32; 4096],
    labels: [u16; 4096],
    len: usize,
}

#[cfg_attr(verus_only, verus_verify)]
impl Default for PackedRectangles64 {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.initialized(), result@ == Seq::<Rectangle>::empty()))]
    fn default() -> Self {
        Self::EMPTY
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl PackedRectangles64 {
    #[cfg_attr(verus_only, verifier::exec)]
    #[cfg_attr(verus_only, verus_spec(ensures Self::EMPTY.valid(), Self::EMPTY.initialized(), Self::EMPTY@ == Seq::<Rectangle>::empty()))]
    const EMPTY: Self = {
        let output = Self {
            bounds: [0; 4096],
            labels: [0; 4096],
            len: 0,
        };
        #[cfg(verus_only)]
        proof! { proof::zero_storage(&output); }
        output
    };

    /// 创建空输出并初始化全部固定存储，不申请堆内存。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result.valid(), result.initialized(), result@ == Seq::<Rectangle>::empty()))]
    pub const fn new() -> Self {
        Self::EMPTY
    }

    /// 清空有效长度，保留并复用已有字节。
    #[cfg_attr(verus_only, verus_spec(
        ensures final(self).valid(), final(self)@ == Seq::<Rectangle>::empty(),
            final(self).storage() == old(self).storage(),
    ))]
    pub const fn clear(&mut self) {
        self.len = 0;
    }

    /// 有效矩形数，不超过 4096。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == self@.len(), result <= 4096))]
    pub const fn len(&self) -> usize {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); }
        self.len
    }

    /// 当前是否没有有效矩形。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result => ensures result == (self@.len() == 0)))]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 有效矩形边界；四个低到高字节为 x 起止、y 起止，均为闭开范围。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result@ == self.bounds_view(), result@.len() == self@.len(),
    ))]
    pub const fn bounds(&self) -> &[u32] {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); }
        self.bounds.split_at(self.len).0
    }

    /// 与 bounds 同索引的非零标签。
    #[must_use]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result@ == self.labels_view(), result@.len() == self@.len(),
            forall|i: int| #![auto] 0 <= i < self@.len() ==> result@[i] == self@[i].value,
    ))]
    pub const fn labels(&self) -> &[u16] {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); }
        self.labels.split_at(self.len).0
    }

    /// 自身固定容量的总字节数，不包含消费者解码后的输出。
    pub const fn retained_bytes(&self) -> usize {
        size_of::<Self>()
    }

    /// 按有效前缀逐项解码，不分配或修改存储；剩余数量与顺序精确对应当前视图。
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result.obeys_prophetic_iter_laws(), result.remaining() == self@,
    ))]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Rectangle> + '_ {
        #[cfg(verus_only)]
        proof! { use_type_invariant(self); }
        PackedIter {
            bounds: self.bounds(),
            labels: self.labels(),
        }
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        requires old(self).valid(),
        ensures final(self).valid(), proof::pushed(old(self), final(self), rectangle, result),
            (result is Ok) == (old(self)@.len() < 4096),
            final(self)@ == if result is Ok { old(self)@.push(rectangle) } else { old(self)@ },
    ))]
    pub(crate) fn push(
        &mut self,
        rectangle: Rectangle,
    ) -> Result<(), crate::error::SparseQuadError> {
        if self.len == self.bounds.len() {
            return Err(crate::error::SparseQuadError::CapacityOverflow);
        }
        #[cfg(verus_only)]
        proof! { proof::encoding_roundtrip(rectangle, 0, 0); }
        *crate::index::get_mut(&mut self.bounds, self.len) = rectangle.packed_bounds();
        *crate::index::get_mut(&mut self.labels, self.len) = rectangle.value;
        self.len += 1;
        Ok(())
    }
}

/// 同一有效前缀的两个借用切片；推进时同步缩短，不保留旧输出或分配。
#[cfg_attr(verus_only, verus_verify)]
struct PackedIter<'a> {
    bounds: &'a [u32],
    labels: &'a [u16],
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> Iterator for PackedIter<'a> {
    type Item = Rectangle;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        #[cfg(verus_only)]
        proof! { use_type_invariant(&*self); }
        let (Some((&bounds, remaining_bounds)), Some((&value, remaining_labels))) =
            (self.bounds.split_first(), self.labels.split_first())
        else {
            return None;
        };
        *self = Self {
            bounds: remaining_bounds,
            labels: remaining_labels,
        };
        Some(Rectangle::from_packed(bounds, value))
    }

    #[inline]
    #[cfg_attr(verus_only, verus_spec(result =>
        ensures result == (self.remaining().len() as usize, Some(self.remaining().len() as usize)),
    ))]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.bounds.len();
        (len, Some(len))
    }
}

#[cfg_attr(verus_only, verus_verify)]
impl<'a> ExactSizeIterator for PackedIter<'a> {
    #[inline]
    fn len(&self) -> usize {
        self.bounds.len()
    }
}

/// 方向枚举。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

/// 有效 chord（矩形边片段），用于 chord 提取和匹配阶段。
///
/// 水平 chord 覆盖 `x1..x2`，垂直 chord 覆盖 `y1..y2`。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
pub struct EffectiveChord {
    pub(crate) orientation: Orientation,
    pub(crate) x1: u8,
    pub(crate) y1: u8,
    pub(crate) x2: u8,
    pub(crate) y2: u8,
}

/// 有效前缀中的带标签 chord；填充槽可以为零，读取由生产者证明标签非零。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
pub(crate) struct ValuedChord {
    pub(crate) value: u16,
    pub(crate) chord: EffectiveChord,
}

#[cfg_attr(verus_only, verus_verify)]
impl ValuedChord {
    #[inline]
    #[cfg_attr(verus_only, verus_spec(result => ensures result.value == value,
        result.vertical() == !HORIZONTAL, result.line() == coordinate,
        result.start() == start, result.end() == end,
        if HORIZONTAL { result.chord.y1 == result.chord.y2 } else { result.chord.x1 == result.chord.x2 },
    ))]
    pub(crate) const fn on_axis<const HORIZONTAL: bool>(
        coordinate: u8,
        start: u8,
        end: u8,
        value: u16,
    ) -> Self {
        let chord = if HORIZONTAL {
            EffectiveChord {
                orientation: Orientation::Horizontal,
                x1: start,
                y1: coordinate,
                x2: end,
                y2: coordinate,
            }
        } else {
            EffectiveChord {
                orientation: Orientation::Vertical,
                x1: coordinate,
                y1: start,
                x2: coordinate,
                y2: end,
            }
        };
        Self { value, chord }
    }

    #[cfg_attr(verus_only, verus_spec(result => requires self.value != 0, ensures result == self.chord))]
    pub(crate) fn chord(&self) -> EffectiveChord {
        debug_assert!(self.value != 0);
        self.chord
    }
}

/// 单行上的水平连续段，`x_start..x_end`。
#[derive(Clone, Copy, Default, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
pub struct Run {
    pub(crate) value: u16,
    pub(crate) x_start: u8,
    pub(crate) x_end: u8,
}

#[cfg_attr(verus_only, verus_verify)]
impl Run {
    #[cfg_attr(verus_only, verus_spec(result => ensures result == (self.value as u32)
        | ((self.x_start as u32) << 16) | ((self.x_end as u32) << 24)))]
    pub(crate) const fn match_key(self) -> u32 {
        (self.value as u32) | ((self.x_start as u32) << 16) | ((self.x_end as u32) << 24)
    }
}

/// 向下延伸中的活跃矩形，`x_start..x_end`。
#[derive(Clone, Copy, Default, Eq, PartialEq)]
#[cfg_attr(verus_only, derive(StructuralEq))]
#[cfg_attr(verus_only, verus_verify)]
pub struct ActiveRect {
    pub(crate) value: u16,
    pub(crate) x_start: u8,
    pub(crate) x_end: u8,
    pub(crate) y_start: u8,
}

#[cfg_attr(verus_only, verus_verify)]
impl ActiveRect {
    #[cfg_attr(verus_only, verus_spec(result => ensures result.value == value,
        result.x_start == x_start, result.x_end == x_end, result.y_start == y_start))]
    pub(crate) const fn new(value: u16, x_start: u8, x_end: u8, y_start: u8) -> Self {
        Self {
            value,
            x_start,
            x_end,
            y_start,
        }
    }

    #[cfg_attr(verus_only, verus_spec(result => ensures result == (self.value as u32)
        | ((self.x_start as u32) << 16) | ((self.x_end as u32) << 24)))]
    pub(crate) const fn match_key(self) -> u32 {
        (self.value as u32) | ((self.x_start as u32) << 16) | ((self.x_end as u32) << 24)
    }
}
