//! 迭代 median 分区；比较策略由构建器提供，不读取灯或设备状态。

use std::cmp::Ordering;

/// 原位保留元素并按给定总序划分；调用方保证非空及 `middle < order.len()`。
pub(super) fn select_median(
    order: &mut [u32],
    middle: usize,
    compare: &impl Fn(u32, u32) -> Ordering,
) {
    let mut lo = 0;
    let mut hi = order.len();
    let mut budget = 2 * order.len().ilog2();
    while hi - lo > 16 {
        if budget == 0 {
            heap_sort(&mut order[lo..hi], compare);
            return;
        }
        budget -= 1;
        let mut pivots = [order[lo], order[(lo + hi) / 2], order[hi - 1]];
        if compare(pivots[0], pivots[1]).is_gt() {
            pivots.swap(0, 1);
        }
        if compare(pivots[1], pivots[2]).is_gt() {
            pivots.swap(1, 2);
        }
        if compare(pivots[0], pivots[1]).is_gt() {
            pivots.swap(0, 1);
        }
        let pivot = pivots[1];
        let (mut less, mut at, mut greater) = (lo, lo, hi);
        while at < greater {
            match compare(order[at], pivot) {
                Ordering::Less => {
                    order.swap(less, at);
                    less += 1;
                    at += 1;
                }
                Ordering::Equal => at += 1,
                Ordering::Greater => {
                    greater -= 1;
                    order.swap(at, greater);
                }
            }
        }
        if middle < less {
            hi = less;
        } else if middle >= greater {
            lo = greater;
        } else {
            return;
        }
    }
    for i in lo + 1..hi {
        let mut at = i;
        while at > lo && compare(order[at], order[at - 1]).is_lt() {
            order.swap(at, at - 1);
            at -= 1;
        }
    }
}

fn heap_sort(order: &mut [u32], compare: &impl Fn(u32, u32) -> Ordering) {
    for root in (0..order.len() / 2).rev() {
        sift_down(order, root, compare);
    }
    for end in (1..order.len()).rev() {
        order.swap(0, end);
        sift_down(&mut order[..end], 0, compare);
    }
}

fn sift_down(order: &mut [u32], mut at: usize, compare: &impl Fn(u32, u32) -> Ordering) {
    while at * 2 + 1 < order.len() {
        let mut child = at * 2 + 1;
        if child + 1 < order.len() && compare(order[child], order[child + 1]).is_lt() {
            child += 1;
        }
        if compare(order[at], order[child]).is_ge() {
            break;
        }
        order.swap(at, child);
        at = child;
    }
}
