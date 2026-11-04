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
        let minimum = source.minimum();
        let maximum = source.maximum();
        let low: [i64; 3] = std::array::from_fn(|axis| minimum[axis].max(other.minimum()[axis]));
        let high: [i64; 3] = std::array::from_fn(|axis| maximum[axis].min(other.maximum()[axis]));
        if (0..3).any(|axis| low[axis] >= high[axis]) {
            self.push(minimum, maximum, enter);
            return;
        }
        let mut middle_min = minimum;
        let mut middle_max = maximum;
        for axis in 0..3 {
            let mut upper = middle_max;
            upper[axis] = low[axis];
            self.push(middle_min, upper, enter);
            let mut lower = middle_min;
            lower[axis] = high[axis];
            self.push(lower, middle_max, enter);
            middle_min[axis] = low[axis];
            middle_max[axis] = high[axis];
        }
    }

    fn push(&mut self, minimum: [i64; 3], maximum: [i64; 3], enter: bool) {
        if (0..3).all(|axis| minimum[axis] < maximum[axis]) {
            self.slabs[self.len] = Slab {
                minimum,
                maximum,
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
