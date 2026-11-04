//! 私有 chord 的坐标投影与有效形状；不保存像素图。
use super::{Orientation, ValuedChord};
use vstd::prelude::*;

verus! {

impl ValuedChord {
    pub(crate) open spec fn vertical(&self) -> bool { self.chord.orientation is Vertical }
    pub(crate) open spec fn line(&self) -> int { if self.vertical() { self.chord.x1 as int } else { self.chord.y1 as int } }
    pub(crate) open spec fn start(&self) -> int { if self.vertical() { self.chord.y1 as int } else { self.chord.x1 as int } }
    pub(crate) open spec fn end(&self) -> int { if self.vertical() { self.chord.y2 as int } else { self.chord.x2 as int } }
    pub(crate) open spec fn id(&self) -> int {
        (if self.vertical() { 262144int } else { 0 }) + 4096 * self.line() + 64 * self.start() + self.end()
    }
}

impl ValuedChord {
    pub(crate) open spec fn well_formed(&self, vertical: bool) -> bool {
        self.vertical() == vertical && self.value != 0
            && 1 <= self.line() <= 63 && 1 <= self.start() < self.end() <= 63
            && if vertical { self.chord.x1 == self.chord.x2 } else { self.chord.y1 == self.chord.y2 }
    }
}
}
