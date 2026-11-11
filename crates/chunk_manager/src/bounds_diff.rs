//! LOD0完整有限窗口的对称差；不访问重叠体积和球外行。

use crate::{Bounds, Chunk, RootChange};

#[derive(Clone, Copy)]
struct Slab {
    minimum: [i64; 3],
    maximum: [i64; 3],
    enter: bool,
}

impl Slab {
    const EMPTY: Self = Self {
        minimum: [0; 3],
        maximum: [0; 3],
        enter: false,
    };
}

pub(crate) struct BoundsDiff {
    slabs: [Slab; 12],
    len: usize,
    index: usize,
    current: [i64; 3],
}

impl BoundsDiff {
    pub(crate) fn new(previous: Bounds, current: Bounds) -> Self {
        let mut result = Self {
            slabs: [Slab::EMPTY; 12],
            len: 0,
            index: 0,
            current: [0; 3],
        };
        result.append(previous, current, false);
        result.append(current, previous, true);
        if result.len != 0 {
            result.current = result.slabs[0].minimum;
        }
        result
    }

    fn append(&mut self, source: Bounds, other: Bounds, enter: bool) {
        for bounds in source.difference(other) {
            self.slabs[self.len] = Slab {
                minimum: bounds.minimum(),
                maximum: bounds.maximum(),
                enter,
            };
            self.len += 1;
        }
    }

    pub(crate) fn next(&mut self) -> Option<RootChange> {
        if self.index == self.len {
            return None;
        }
        let slab = self.slabs[self.index];
        let chunk = Chunk::new(self.current, 0);
        let mut advanced = false;
        for axis in 0..3 {
            if self.current[axis] < slab.maximum[axis] - 1 {
                self.current[axis] += 1;
                advanced = true;
                break;
            }
            self.current[axis] = slab.minimum[axis];
        }
        if !advanced {
            self.index += 1;
            if self.index != self.len {
                self.current = self.slabs[self.index].minimum;
            }
        }
        Some(if slab.enter {
            RootChange::Enter(chunk)
        } else {
            RootChange::Exit(chunk)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn subtraction_keeps_extreme_endpoints_without_overflow() {
        let outer = Bounds::new([i64::MIN; 3], [i64::MAX; 3]).unwrap();
        let inner = Bounds::new([-1; 3], [1; 3]).unwrap();
        let parts: Vec<_> = outer.difference(inner).collect();
        assert_eq!(parts.len(), 6);
        for (i, a) in parts.iter().enumerate() {
            for b in parts.iter().skip(i + 1).chain(std::iter::once(&inner)) {
                assert!((0..3).any(|axis| a.maximum()[axis] <= b.minimum()[axis]
                    || b.maximum()[axis] <= a.minimum()[axis]));
            }
        }
        assert_eq!(outer.difference(outer).len(), 0);
    }

    fn points(bounds: Bounds) -> BTreeSet<[i64; 3]> {
        let mut result = BTreeSet::new();
        for x in bounds.minimum()[0]..bounds.maximum()[0] {
            for y in bounds.minimum()[1]..bounds.maximum()[1] {
                for z in bounds.minimum()[2]..bounds.maximum()[2] {
                    result.insert([x, y, z]);
                }
            }
        }
        result
    }

    #[test]
    fn rectangular_difference_matches_endpoint_sets_without_duplicates() {
        let old = Bounds::new([-2; 3], [2; 3]).unwrap();
        for shift in [[0; 3], [1, 0, 0], [-1, 1, -1], [8; 3]] {
            let new = Bounds::new(shift.map(|v| v - 1), shift.map(|v| v + 3)).unwrap();
            let previous = points(old);
            let current = points(new);
            let mut enters = BTreeSet::new();
            let mut exits = BTreeSet::new();
            let mut diff = BoundsDiff::new(old, new);
            while let Some(change) = diff.next() {
                assert!(match change {
                    RootChange::Enter(chunk) => enters.insert(chunk.coord),
                    RootChange::Exit(chunk) => exits.insert(chunk.coord),
                });
            }
            assert_eq!(enters, current.difference(&previous).copied().collect());
            assert_eq!(exits, previous.difference(&current).copied().collect());
        }
    }
}
