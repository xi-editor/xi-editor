// Copyright 2016 Google Inc. All rights reserved.
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

//! Intervals that can be open or closed at the ends.

use std::cmp::{min, max};
use std::fmt;

// Invariant: end >= start
// (attempting to construct an interval of negative size gives an
// empty interval beginning at start)

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Interval {
    start: u64,  // 2 * the actual value + 1 if open
    end: u64,    // 2 * the actual value + 1 if closed
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_start_closed() {
            write!(f, "[")?;
        } else {
            write!(f, "(")?;
        }
        write!(f, "{}, {}", self.start(), self.end())?;
        if self.is_end_closed() {
            write!(f, "]")?;
        } else {
            write!(f, ")")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Interval {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Interval {
    pub fn new(start: usize, start_closed : bool, end: usize, end_closed: bool) -> Interval {
        let start = (start as u64) * 2 + if start_closed { 0 } else { 1 };
        let end = (end as u64) * 2 + if end_closed { 1 } else { 0 };
        Interval {
            start: start,
            end: max(start, end),
        }
    }

    pub fn new_open_open(start: usize, end: usize) -> Interval {
        Self::new(start, false, end, false)
    }

    pub fn new_open_closed(start: usize, end: usize) -> Interval {
        Self::new(start, false, end, true)
    }

    pub fn new_closed_open(start: usize, end: usize) -> Interval {
        Self::new(start, true, end, false)
    }

    pub fn new_closed_closed(start: usize, end: usize) -> Interval {
        Self::new(start, true, end, true)
    }

    pub fn start(&self) -> usize {
        (self.start / 2) as usize
    }

    pub fn end(&self) -> usize {
        (self.end / 2) as usize
    }

    pub fn start_end(&self) -> (usize, usize) {
        (self.start(), self.end())
    }

    pub fn is_start_closed(&self) -> bool {
        (self.start & 1) == 0
    }

    pub fn is_end_closed(&self) -> bool {
        (self.end & 1) != 0
    }

    // The following 3 methods define a trisection, exactly one is true.
    // (similar to std::cmp::Ordering, but "Equal" is not the same as "contains")

    // the interval is before the point (the point is after the interval)
    pub fn is_before(&self, val: usize) -> bool {
        let val2 = (val as u64) * 2;
        self.end <= val2
    }

    // the point is inside the interval
    pub fn contains(&self, val: usize) -> bool {
        let val2 = (val as u64) * 2;
        self.start <= val2 && val2 < self.end
    }

    // the interval is after the point (the point is before the interval)
    pub fn is_after(&self, val: usize) -> bool {
        let val2 = (val as u64) * 2;
        self.start > val2
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    // impl BitAnd would be completely valid for this
    pub fn intersect(&self, other: Interval) -> Interval {
        let start = max(self.start, other.start);
        let end = min(self.end, other.end);
        Interval {
            start: start,
            end: max(start, end),
        }
    }

    // smallest interval that encloses both inputs; if the inputs are
    // disjoint, then it fills in the hole.
    pub fn union(&self, other: Interval) -> Interval {
        if self.is_empty() { return other; }
        if other.is_empty() { return *self; }
        let start = min(self.start, other.start);
        let end = max(self.end, other.end);
        Interval {
            start: start,
            end: end,
        }
    }

    // the first half of self - other
    pub fn prefix(&self, other: Interval) -> Interval {
        Interval {
            start: min(self.start, other.start),
            end: min(self.end, other.start),
        }
    }

    // the second half of self - other
    pub fn suffix(&self, other: Interval) -> Interval {
        Interval {
            start: max(self.start, other.end),
            end: max(self.end, other.end),
        }
    }

    // could impl Add trait, but that's probably too cute
    pub fn translate(&self, amount: usize) -> Interval {
        let amount2 = (amount as u64) * 2;
        Interval {
            start: self.start + amount2,
            end: self.end + amount2,
        }
    }

    // as above for Sub trait
    pub fn translate_neg(&self, amount: usize) -> Interval {
        let amount2 = (amount as u64) * 2;
        Interval {
            start: self.start - amount2,
            end: self.end - amount2,
        }
    }

    // insensitive to open or closed ends, just the size of the interior
    pub fn size(&self) -> usize {
        self.end() - self.start()
    }
}

#[cfg(test)]
mod tests {
    use interval::Interval;

    #[test]
    fn new_params() {
        let i = Interval::new(2, false, 42, false);
        assert_eq!(2, i.start());
        assert!(!i.is_start_closed());
        assert_eq!(42, i.end());
        assert!(!i.is_end_closed());

        let i = Interval::new(2, false, 42, true);
        assert_eq!(2, i.start());
        assert!(!i.is_start_closed());
        assert_eq!(42, i.end());
        assert!(i.is_end_closed());

        let i = Interval::new(2, true, 42, false);
        assert_eq!(2, i.start());
        assert!(i.is_start_closed());
        assert_eq!(42, i.end());
        assert!(!i.is_end_closed());

        let i = Interval::new(2, true, 42, true);
        assert_eq!(2, i.start());
        assert!(i.is_start_closed());
        assert_eq!(42, i.end());
        assert!(i.is_end_closed());
    }

    #[test]
    fn new_variants() {
        let i = Interval::new_open_open(2, 42);
        assert_eq!(i, Interval::new(2, false, 42, false));

        let i = Interval::new_open_closed(2, 42);
        assert_eq!(i, Interval::new(2, false, 42, true));

        let i = Interval::new_closed_open(2, 42);
        assert_eq!(i, Interval::new(2, true, 42, false));

        let i = Interval::new_closed_closed(2, 42);
        assert_eq!(i, Interval::new(2, true, 42, true));
    }

    #[test]
    fn contains() {
        let i = Interval::new_open_open(2, 42);
        assert!(!i.contains(1));
        assert!(!i.contains(2));
        assert!(i.contains(3));
        assert!(i.contains(41));
        assert!(!i.contains(42));
        assert!(!i.contains(43));

        let i = Interval::new_closed_closed(2, 42);
        assert!(!i.contains(1));
        assert!(i.contains(2));
        assert!(i.contains(3));
        assert!(i.contains(41));
        assert!(i.contains(42));
        assert!(!i.contains(43));
    }

    #[test]
    fn before() {
        let i = Interval::new_open_open(2, 42);
        assert!(!i.is_before(1));
        assert!(!i.is_before(2));
        assert!(!i.is_before(3));
        assert!(!i.is_before(41));
        assert!(i.is_before(42));
        assert!(i.is_before(43));

        let i = Interval::new_closed_closed(2, 42);
        assert!(!i.is_before(1));
        assert!(!i.is_before(2));
        assert!(!i.is_before(3));
        assert!(!i.is_before(41));
        assert!(!i.is_before(42));
        assert!(i.is_before(43));
    }

    #[test]
    fn after() {
        let i = Interval::new_open_open(2, 42);
        assert!(i.is_after(1));
        assert!(i.is_after(2));
        assert!(!i.is_after(3));
        assert!(!i.is_after(41));
        assert!(!i.is_after(42));
        assert!(!i.is_after(43));

        let i = Interval::new_closed_closed(2, 42);
        assert!(i.is_after(1));
        assert!(!i.is_after(2));
        assert!(!i.is_after(3));
        assert!(!i.is_after(41));
        assert!(!i.is_after(42));
        assert!(!i.is_after(43));
    }

    #[test]
    fn translate() {
        let i = Interval::new_open_open(2, 42);
        assert_eq!(Interval::new_open_open(5, 45), i.translate(3));
        assert_eq!(Interval::new_open_open(1, 41), i.translate_neg(1));
    }

    #[test]
    fn empty() {
        assert!(Interval::new_closed_open(0, 0).is_empty());
        assert!(!Interval::new_closed_closed(0, 0).is_empty());
        assert!(!Interval::new_closed_open(0, 1).is_empty());

        assert!(Interval::new_open_open(1, 0).is_empty());
    }

    #[test]
    fn intersect() {
        assert_eq!(Interval::new_closed_open(2, 3),
            Interval::new_open_open(1, 3).intersect(
                Interval::new_closed_closed(2, 4)));
        assert!(Interval::new_closed_open(1, 2).intersect(
            Interval::new_closed_closed(2, 43))
                .is_empty());
    }

    #[test]
    fn prefix() {
        assert_eq!(Interval::new_open_open(1, 2),
            Interval::new_open_open(1, 4).prefix(
                Interval::new_closed_closed(2, 3)));
    }

    #[test]
    fn suffix() {
        assert_eq!(Interval::new_open_open(3, 4),
            Interval::new_open_open(1, 4).suffix(
                Interval::new_closed_closed(2, 3)));
    }

    #[test]
    fn size() {
        assert_eq!(40, Interval::new_closed_open(2, 42).size());
        assert_eq!(0, Interval::new_closed_open(1, 0).size());
    }
}
