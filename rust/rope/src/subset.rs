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

//! A data structure for representing subsets of sequences (typically strings).

use std::cmp::{max};

// These two imports are for the `apply` method only.
use tree::{Node, NodeInfo, TreeBuilder};
use interval::Interval;
use std::slice;

// Internally, a sorted list of (begin, end) ranges.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Subset(Vec<(usize, usize)>);

#[derive(Default)]
pub struct SubsetBuilder {
    ranges: Vec<(usize, usize)>,
    b: usize,
    e: usize,
}

impl SubsetBuilder {
    pub fn new() -> SubsetBuilder {
        SubsetBuilder::default()
    }

    pub fn add_range(&mut self, beg: usize, end: usize) {
        if beg > self.e {
            if self.e > self.b {
                self.ranges.push((self.b, self.e));
            }
            self.b = beg
        }
        self.e = end;
    }

    pub fn build(mut self) -> Subset {
        if self.e > self.b {
            self.ranges.push((self.b, self.e));
        }
        Subset(self.ranges)
    }
}

impl Subset {
    /// Mostly for testing.
    pub fn delete_from_string(&self, s: &str) -> String {
        let mut result = String::new();
        for (b, e) in self.complement_iter(s.len()) {
            result.push_str(&s[b..e]);
        }
        result
    }

    // Maybe Subset should be a pure data structure and this method should
    // be a method of Node.
    /// Builds a version of `s` with all the elements in this `Subset` deleted from it.
    pub fn delete_from<N: NodeInfo>(&self, s: &Node<N>) -> Node<N> {
        let mut b = TreeBuilder::new();
        for (beg, end) in self.complement_iter(s.len()) {
            s.push_subseq(&mut b, Interval::new_closed_open(beg, end));
        }
        b.build()
    }

    /// The length of the resulting sequence after deleting this subset.
    ///
    /// `self.delete_from_string(s).len() = self.len(s.len())`
    pub fn len_after_delete(&self, base_len: usize) -> usize {
        self.0.iter().fold(base_len, |acc, &(b, e)| acc - (e - b))
    }

    /// Determine whether the subset is empty.
    /// In this case deleting it would do nothing.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[doc(hidden)]
    // Access to internal state, shouldn't really be part of public API
    pub fn _ranges(&self) -> &[(usize, usize)] {
        &self.0
    }

    /// Compute the union of two subsets. In other words, an element exists in the
    /// resulting subset iff it exists in at least one of the inputs.
    pub fn union(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut i = 0;
        let mut j = 0;
        loop {
            let (next_beg, mut next_end) = if i == self.0.len() {
                if j == other.0.len() {
                    break;
                } else {
                    let del = other.0[j];
                    j += 1;
                    del
                }
            } else if j == other.0.len() || self.0[i].0 < other.0[j].0 {
                let del = self.0[i];
                i += 1;
                del
            } else {
                let del = other.0[j];
                j += 1;
                del
            };
            loop {
                if i < self.0.len() && self.0[i].0 <= next_end {
                    next_end = max(next_end, self.0[i].1);
                    i += 1;
                    continue;
                } else if j < other.0.len() && other.0[j].0 <= next_end {
                    next_end = max(next_end, other.0[j].1);
                    j += 1;
                    continue;
                } else {
                    break;
                }
            }
            sb.add_range(next_beg, next_end);
        }
        sb.build()
    }

    /// Transform through coordinate transform represented by other.
    /// The equation satisfied is as follows:
    ///
    /// s1 = other.delete_from_string(s0)
    ///
    /// s2 = self.delete_from_string(s1)
    ///
    /// element in self.transform_expand(other).delete_from_string(s0) if (not in s1) or in s2
    pub fn transform_expand(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut last = 0;
        let mut i = 0;
        let mut delta = 0;
        for &(b, e) in &other.0 {
            loop {
                if i >= self.0.len() {
                    // early exit, no more deletions will happen
                    return sb.build();
                }
                if self.0[i].1 + delta < b {
                    sb.add_range(max(last, self.0[i].0 + delta), self.0[i].1 + delta);
                    i += 1;
                } else {
                    break;
                }
            }
            if self.0[i].0 + delta < b {
                sb.add_range(max(last, self.0[i].0 + delta), b);
            }
            last = e;
            delta += e - b;
        }
        if i < self.0.len() && self.0[i].0 + delta < last {
            sb.add_range(last, self.0[i].1 + delta);
            i += 1;
        }
        for &(b, e) in &self.0[i..] {
            sb.add_range(b + delta, e + delta);
        }
        sb.build()
    }

    /// The same as taking transform_expand and then unioning with `other`.
    pub fn transform_union(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut last = 0;
        let mut i = 0;
        let mut delta = 0;
        for &(b, e) in &other.0 {
            while i < self.0.len() && self.0[i].1 + delta < b {
                sb.add_range(max(last, self.0[i].0 + delta), self.0[i].1 + delta);
                i += 1;
            }
            if i < self.0.len() && self.0[i].0 + delta < b {
                sb.add_range(max(last, self.0[i].0 + delta), b);
            }
            sb.add_range(b, e);
            last = e;
            delta += e - b;
        }
        if i < self.0.len() && self.0[i].0 + delta < last {
            sb.add_range(last, self.0[i].1 + delta);
            i += 1;
        }
        for &(b, e) in &self.0[i..] {
            sb.add_range(b + delta, e + delta);
        }
        sb.build()
    }

    /// Transform subset through other coordinate transform, shrinking.
    /// The following equation is satisfied:
    ///
    /// C = A.transform_expand(B)
    ///
    /// C.transform_shrink(B).delete_from_string(C.delete_from_string(s)) =
    ///   A.delete_from_string(B.delete_from_string(s))
    pub fn transform_shrink(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut last = 0;
        let mut i = 0;
        let mut y = 0;
        for &(b, e) in &self.0 {
            if i < other.0.len() && other.0[i].0 < last && other.0[i].1 < b {
                sb.add_range(y, other.0[i].1 + y - last);
                i += 1;
            }
            while i < other.0.len() && other.0[i].1 < b {
                sb.add_range(other.0[i].0 + y - last, other.0[i].1 + y - last);
                i += 1;
            }
            if i < other.0.len() && other.0[i].0 < b {
                sb.add_range(max(last, other.0[i].0) + y - last, b + y - last);
            }
            while i < other.0.len() && other.0[i].1 < e {
                i += 1;
            }
            y += b - last;
            last = e;
        }
        if i < other.0.len() && other.0[i].0 < last {
            sb.add_range(y, other.0[i].1 + y - last);
            i += 1;
        }
        for &(b, e) in &other.0[i..] {
            sb.add_range(b + y - last, e + y - last);
        }
        sb.build()
    }

    /// Return an iterator over the ranges not in the Subset. These will
    /// often be easier to work with if the raw ranges are deletions.
    pub fn complement_iter(&self, base_len: usize) -> ComplementIter {
        ComplementIter {
            ranges: &self.0,
            base_len: base_len,
            i: 0,
            last: 0,
        }
    }

    /// Find the complement of this Subset, every element in the subset will
    /// be excluded and vice versa.
    pub fn complement(&self, base_len: usize) -> Subset {
        Subset(self.complement_iter(base_len).collect())
    }

    /// Return a `Mapper` that can be use to map coordinates in the document to coordinates
    /// in this `Subset`, but only in non-decreasing order for performance reasons.
    pub fn mapper(&self) -> Mapper {
        Mapper {
            range_iter: self.0.iter(),
            last_i: 0, // indices only need to be in non-decreasing order, not increasing
            cur_range: (0,0), // will immediately try to consume next range
            subset_amount_consumed: 0,
        }
    }
}

pub struct ComplementIter<'a> {
    ranges: &'a [(usize, usize)],
    base_len: usize,
    i: usize,
    last: usize,
}

impl<'a> Iterator for ComplementIter<'a> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        loop {
            if self.i == self.ranges.len() {
                if self.base_len > self.last {
                    let beg = self.last;
                    self.last = self.base_len;
                    return Some((beg, self.base_len));
                } else {
                    return None;
                }
            } else {
                let beg = self.last;
                let (end, last) = self.ranges[self.i];
                self.last = last;
                self.i += 1;
                if end > beg {
                    return Some((beg, end));
                }
            }
        }
    }
}

pub struct Mapper<'a> {
    range_iter: slice::Iter<'a, (usize,usize)>,
    // Not actually necessary for computation, just for dynamic checking of invariant
    last_i: usize,
    cur_range: (usize,usize),
    subset_amount_consumed: usize,
}

impl<'a> Mapper<'a> {
    /// Map a coordinate in the document this subset corresponds to, to a
    /// coordinate in the subset. For example, if the Subset is a set of
    /// deletions, this would map indices in the union string to indices in
    /// the tombstones string.
    ///
    /// Will return the closest coordinate in the subset if the index is not
    /// in the subset. If the coordinate is past the end of the subset it will
    /// return one more than the largest index in the subset (i.e the length).
    /// This behaviour is suitable for mapping closed-open intervals in a
    /// string to intervals in a subset of the string.
    ///
    /// In order to guarantee good performance, this method must be called
    /// with `i` values in non-decreasing order or it will panic. This allows
    /// the total cost to be O(n) where `n = max(calls,ranges)` over all times
    /// called on a single `Mapper`.
    #[inline]
    pub fn doc_index_to_subset(&mut self, i: usize) -> usize {
        assert!(i >= self.last_i, "method must be called with i in non-decreasing order. i={}<{}=last_i", i, self.last_i);
        self.last_i = i;

        while i >= self.cur_range.1 {
            self.subset_amount_consumed += self.cur_range.1 - self.cur_range.0;
            self.cur_range = match self.range_iter.next() {
                Some(range) => range.clone(),
                // past the end of the subset
                None => {
                    // ensure we don't try to consume any more
                    self.cur_range = (usize::max_value(), usize::max_value());
                    return self.subset_amount_consumed
                }
            }
        }

        if i >= self.cur_range.0 {
            let dist_in_range = i - self.cur_range.0;
            dist_in_range + self.subset_amount_consumed
        } else { // not in the subset
            self.subset_amount_consumed
        }
    }
}

#[cfg(test)]
mod tests {
    use subset::{Subset, SubsetBuilder};

    const TEST_STR: &'static str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    fn find_deletions(substr: &str, s: &str) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut j = 0;
        for i in 0..s.len() {
            if j < substr.len() && substr.as_bytes()[j] == s.as_bytes()[i] {
                j += 1;
            } else {
                sb.add_range(i, i + 1);
            }
        }
        sb.build()
    }

    #[test]
    fn test_apply() {
        let mut sb = SubsetBuilder::new();
        for &(b, e) in &[(0, 1), (2, 4), (6, 11), (13, 14), (15, 18), (19, 23), (24, 26), (31, 32),
                (33, 35), (36, 37), (40, 44), (45, 48), (49, 51), (52, 57), (58, 59)] {
            sb.add_range(b, e);
        }
        let s = sb.build();
        assert_eq!("145BCEINQRSTUWZbcdimpvxyz", s.delete_from_string(TEST_STR));
    }

    #[test]
    fn trivial() {
        let s = SubsetBuilder::new().build();
        assert!(s.is_empty());
    }

    #[test]
    fn test_find_deletions() {
        let substr = "015ABDFHJOPQVYdfgloprsuvz";
        let s = find_deletions(substr, TEST_STR);
        assert_eq!(substr, s.delete_from_string(TEST_STR));
        assert!(!s.is_empty())
    }

    #[test]
    fn test_complement() {
        let substr = "0456789DEFGHIJKLMNOPQRSTUVWXYZdefghijklmnopqrstuvw";
        let s = find_deletions(substr, TEST_STR);
        let c = s.complement(TEST_STR.len());
        // deleting the complement of the deletions we found should yield the deletions
        assert_eq!("123ABCabcxyz", c.delete_from_string(TEST_STR));
    }

    #[test]
    fn test_mapper() {
        let substr = "469ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwz";
        let s = find_deletions(substr, TEST_STR);
        let mut m = s.mapper();
        // subset is {0123 5 78 xy}
        assert_eq!(0, m.doc_index_to_subset(0));
        assert_eq!(2, m.doc_index_to_subset(2));
        assert_eq!(2, m.doc_index_to_subset(2));
        assert_eq!(3, m.doc_index_to_subset(3));
        assert_eq!(4, m.doc_index_to_subset(4)); // not in subset
        assert_eq!(4, m.doc_index_to_subset(5));
        assert_eq!(5, m.doc_index_to_subset(7));
        assert_eq!(6, m.doc_index_to_subset(8));
        assert_eq!(6, m.doc_index_to_subset(8));
        assert_eq!(8, m.doc_index_to_subset(60));
        assert_eq!(9, m.doc_index_to_subset(61)); // not in subset
        assert_eq!(9, m.doc_index_to_subset(62)); // not in subset
    }

    #[test]
    #[should_panic(expected = "non-decreasing")]
    fn test_mapper_requires_non_decreasing() {
        let substr = "469ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvw";
        let s = find_deletions(substr, TEST_STR);
        let mut m = s.mapper();
        m.doc_index_to_subset(0);
        m.doc_index_to_subset(2);
        m.doc_index_to_subset(1);
    }

    #[test]
    fn union() {
        let s1 = find_deletions("024AEGHJKNQTUWXYZabcfgikqrvy", TEST_STR);
        let s2 = find_deletions("14589DEFGIKMOPQRUXZabcdefglnpsuxyz", TEST_STR);
        assert_eq!("4EGKQUXZabcfgy", s1.union(&s2).delete_from_string(TEST_STR));
    }

    fn transform_case(str1: &str, str2: &str, result: &str) {
        let s1 = find_deletions(str1, TEST_STR);
        let s2 = find_deletions(str2, str1);
        let s3 = s2.transform_expand(&s1);
        let str3 = s3.delete_from_string(TEST_STR);
        assert_eq!(result, str3);
        assert_eq!(str2, s3.transform_shrink(&s1).delete_from_string(&str3));
        assert_eq!(str2, s2.transform_union(&s1).delete_from_string(TEST_STR));
    }

    #[test]
    fn transform() {
        transform_case("02345678BCDFGHKLNOPQRTUVXZbcefghjlmnopqrstwx", "027CDGKLOTUbcegopqrw",
            "01279ACDEGIJKLMOSTUWYabcdegikopqruvwyz");
        transform_case("01234678DHIKLMNOPQRUWZbcdhjostvy", "136KLPQZvy",
            "13569ABCEFGJKLPQSTVXYZaefgiklmnpqruvwxyz");
        transform_case("0125789BDEFIJKLMNPVXabdjmrstuwy", "12BIJVXjmrstu",
            "12346ABCGHIJOQRSTUVWXYZcefghijklmnopqrstuvxz");
        transform_case("12456789ABCEFGJKLMNPQRSTUVXYadefghkrtwxz", "15ACEFGKLPRUVYdhrtx",
            "0135ACDEFGHIKLOPRUVWYZbcdhijlmnopqrstuvxy");
        transform_case("0128ABCDEFGIJMNOPQXYZabcfgijkloqruvy", "2CEFGMZabijloruvy",
            "2345679CEFGHKLMRSTUVWZabdehijlmnoprstuvwxyz");
        transform_case("01245689ABCDGJKLMPQSTWXYbcdfgjlmnosvy", "01245ABCDJLQSWXYgsv",
            "0123457ABCDEFHIJLNOQRSUVWXYZaeghikpqrstuvwxz");
    }
}
