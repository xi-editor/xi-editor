// Copyright 2016 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Closed-open intervals, and operations on them.

use std::{
    cmp::{max, min},
    ops::{Range, RangeFrom, RangeFull, RangeInclusive, RangeTo, RangeToInclusive},
};

pub trait Interval {
    fn is_before(&self, val: usize) -> bool;
    fn is_after(&self, val: usize) -> bool;
    fn intersect(&self, other: &Self) -> Self;
    fn union(&self, other: &Self) -> Self;
    fn prefix(&self, other: &Self) -> Self;
    fn suffix(&self, other: &Self) -> Self;
    fn translate(&self, amount: usize) -> Self;
    fn translate_neg(&self, amount: usize) -> Self;
    fn size(&self) -> usize;
}
impl Interval for Range<usize> {
    // The following 2 methods define a trisection, exactly one is true.

    /// the interval is before the point (the point is after the interval)
    fn is_before(&self, val: usize) -> bool {
        self.end <= val
    }

    /// the interval is after the point (the point is before the interval)
    fn is_after(&self, val: usize) -> bool {
        self.start > val
    }

    // impl BitAnd would be completely valid for this
    fn intersect(&self, other: &Self) -> Self {
        let start = max(self.start, other.start);
        let end = min(self.end, other.end);
        Range { start, end: max(start, end) }
    }

    // smallest interval that encloses both inputs; if the inputs are
    // disjoint, then it fills in the hole.
    fn union(&self, other: &Self) -> Self {
        if self.is_empty() {
            return other.clone();
        }
        if other.is_empty() {
            return self.clone();
        }
        let start = min(self.start, other.start);
        let end = max(self.end, other.end);
        Range { start, end }
    }

    // the first half of self - other
    fn prefix(&self, other: &Self) -> Self {
        Range { start: min(self.start, other.start), end: min(self.end, other.start) }
    }

    // the second half of self - other
    fn suffix(&self, other: &Self) -> Self {
        Range { start: max(self.start, other.end), end: max(self.end, other.end) }
    }

    // could impl Add trait, but that's probably too cute
    fn translate(&self, amount: usize) -> Self {
        Range { start: self.start + amount, end: self.end + amount }
    }

    // as above for Sub trait
    fn translate_neg(&self, amount: usize) -> Self {
        debug_assert!(self.start >= amount);
        Range { start: self.start - amount, end: self.end - amount }
    }

    // insensitive to open or closed ends, just the size of the interior
    fn size(&self) -> usize {
        self.end - self.start
    }
}

/// A trait for types that represent unbounded ranges; they need an explicit
/// upper bound in order to be converted to `Interval`s.
///
/// This exists so that some methods that use `Interval` under the hood can
/// accept arguments like `..` or `10..`.
///
/// This trait should only be used when the idea of taking all of something
/// makes sense.
pub trait IntervalBounds {
    fn into_interval(self, upper_bound: usize) -> Range<usize>;
}

impl IntervalBounds for Range<usize> {
    fn into_interval(self, _upper_bound: usize) -> Range<usize> {
        self
    }
}

impl IntervalBounds for RangeFrom<usize> {
    fn into_interval(self, upper_bound: usize) -> Range<usize> {
        Range { start: self.start, end: upper_bound }
    }
}

impl IntervalBounds for RangeFull {
    fn into_interval(self, upper_bound: usize) -> Range<usize> {
        Range { start: 0, end: upper_bound }
    }
}
impl IntervalBounds for RangeTo<usize> {
    fn into_interval(self, _upper_bound: usize) -> Range<usize> {
        Range { start: 0, end: self.end }
    }
}

impl IntervalBounds for RangeInclusive<usize> {
    fn into_interval(self, _upper_bound: usize) -> Range<usize> {
        Range { start: *self.start(), end: self.end().saturating_add(1) }
    }
}

impl IntervalBounds for RangeToInclusive<usize> {
    fn into_interval(self, _upper_bound: usize) -> Range<usize> {
        Range { start: 0, end: self.end.saturating_add(1) }
    }
}

#[cfg(test)]
mod tests {
    use crate::interval::Interval;

    #[test]
    fn contains() {
        let i = 2..42;
        assert!(!i.contains(&1));
        assert!(i.contains(&2));
        assert!(i.contains(&3));
        assert!(i.contains(&41));
        assert!(!i.contains(&42));
        assert!(!i.contains(&43));
    }

    #[test]
    fn before() {
        let i = 2..42;
        assert!(!i.is_before(1));
        assert!(!i.is_before(2));
        assert!(!i.is_before(3));
        assert!(!i.is_before(41));
        assert!(i.is_before(42));
        assert!(i.is_before(43));
    }

    #[test]
    fn after() {
        let i = 2..42;
        assert!(i.is_after(1));
        assert!(!i.is_after(2));
        assert!(!i.is_after(3));
        assert!(!i.is_after(41));
        assert!(!i.is_after(42));
        assert!(!i.is_after(43));
    }

    #[test]
    fn translate() {
        let i = 2..42;
        assert_eq!(5..45, i.translate(3));
        assert_eq!(1..41, i.translate_neg(1));
    }

    #[test]
    fn empty() {
        assert!((0..0).is_empty());
        assert!((1..1).is_empty());
        assert!(!(1..2).is_empty());
    }

    #[test]
    fn intersect() {
        assert_eq!((2..3), (1..3).intersect(&(2..4)));
        assert!((1..2).intersect(&(2..43)).is_empty());
    }

    #[test]
    fn prefix() {
        assert_eq!((1..2), (1..4).prefix(&(2..3)));
    }

    #[test]
    fn suffix() {
        assert_eq!((3..4), (1..4).suffix(&(2..3)));
    }

    #[test]
    fn size() {
        assert_eq!(40, (2..42).size());
        assert_eq!(0, (1..1).size());
        assert_eq!(1, (1..2).size());
    }
}
