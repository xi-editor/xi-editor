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

// Internally, a sorted list of (begin, end) ranges.
#[derive(Clone, Default)]
pub struct Subset(Vec<(usize, usize)>);

#[derive(Default)]
pub struct SubsetBuilder {
    dels: Vec<(usize, usize)>,
    b: usize,
    e: usize,
}

impl SubsetBuilder {
    pub fn new() -> SubsetBuilder {
        SubsetBuilder::default()
    }

    pub fn add_deletion(&mut self, beg: usize, end: usize) {
        if beg > self.e {
            if self.e > self.b {
                self.dels.push((self.b, self.e));
            }
            self.b = beg
        }
        self.e = end;
    }

    pub fn build(mut self) -> Subset {
        if self.e > self.b {
            self.dels.push((self.b, self.e));
        }
        Subset(self.dels)
    }
}

impl Subset {
    // mostly for testing
    pub fn apply_to_string(&self, s: &str) -> String {
        let mut result = String::new();
        for (b, e) in self.range_iter(s.len()) {
            result.push_str(&s[b..e]);
        }
        result
    }

    // Maybe Subset should be a pure data structure and this method should
    // be a method of Node.
    pub fn apply<N: NodeInfo>(&self, s: &Node<N>) -> Node<N> {
        let mut b = TreeBuilder::new();
        for (beg, end) in self.range_iter(s.len()) {
            s.push_subseq(&mut b, Interval::new_closed_open(beg, end));
        }
        b.build()
    }

    /// The length of the resulting sequence.
    ///
    /// `self.apply_to_string(s).len() = self.len(s.len())`
    pub fn len(&self, base_len: usize) -> usize {
        self.0.iter().fold(base_len, |acc, &(b, e)| acc - (e - b))
    }

    /// Determine whether the subset is trivial, ie applying it does not
    /// change the sequence. In purely set theoretic terms, this is the
    /// same as testing equality with the universal set.
    pub fn is_trivial(&self) -> bool {
        self.0.is_empty()
    }

    #[doc(hidden)]
    // Access to internal state, shouldn't really be part of public API
    // Perhaps exposing an iterator over deleted regions would be more suitable,
    // but it's more of a hassle.
    pub fn _deletions(&self) -> &[(usize, usize)] {
        &self.0
    }

    /// Compute the intersection of two subsets. In other words, an element exists in the
    /// resulting subset iff it exists in both inputs.
    pub fn intersect(&self, other: &Subset) -> Subset {
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
            sb.add_deletion(next_beg, next_end);
        }
        sb.build()
    }

    /// Transform through coordinate transform represented by other.
    /// The equation satisfied is as follows:
    ///
    /// s1 = other.apply_to_string(s0)
    ///
    /// s2 = self.apply_to_string(s1)
    ///
    /// element in self.transform_expand(other).apply_to_string(s0) if not in s1 or in s2
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
                    sb.add_deletion(max(last, self.0[i].0 + delta), self.0[i].1 + delta);
                    i += 1;
                } else {
                    break;
                }
            }
            if self.0[i].0 + delta < b {
                sb.add_deletion(max(last, self.0[i].0 + delta), b);
            }
            last = e;
            delta += e - b;
        }
        if i < self.0.len() && self.0[i].0 + delta < last {
            sb.add_deletion(last, self.0[i].1 + delta);
            i += 1;
        }
        for &(b, e) in &self.0[i..] {
            sb.add_deletion(b + delta, e + delta);
        }
        sb.build()
    }

    /// The same as taking transform_expand and then intersecting with `other`.
    pub fn transform_intersect(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut last = 0;
        let mut i = 0;
        let mut delta = 0;
        for &(b, e) in &other.0 {
            while i < self.0.len() && self.0[i].1 + delta < b {
                sb.add_deletion(max(last, self.0[i].0 + delta), self.0[i].1 + delta);
                i += 1;
            }
            if i < self.0.len() && self.0[i].0 + delta < b {
                sb.add_deletion(max(last, self.0[i].0 + delta), b);
            }
            sb.add_deletion(b, e);
            last = e;
            delta += e - b;
        }
        if i < self.0.len() && self.0[i].0 + delta < last {
            sb.add_deletion(last, self.0[i].1 + delta);
            i += 1;
        }
        for &(b, e) in &self.0[i..] {
            sb.add_deletion(b + delta, e + delta);
        }
        sb.build()   
    }

    /// Transform subset through other coordinate transform, shrinking.
    /// The following equation is satisfied:
    ///
    /// C = A.transform_expand(B)
    ///
    /// C.transform_shrink(B).apply_to_string(C.apply_to_string(s)) =
    ///   A.apply_to_string(B.apply_to_string(s))
    pub fn transform_shrink(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut last = 0;
        let mut i = 0;
        let mut y = 0;
        for &(b, e) in &self.0 {
            if i < other.0.len() && other.0[i].0 < last && other.0[i].1 < b {
                sb.add_deletion(y, other.0[i].1 + y - last);
                i += 1;
            }
            while i < other.0.len() && other.0[i].1 < b {
                sb.add_deletion(other.0[i].0 + y - last, other.0[i].1 + y - last);
                i += 1;
            }
            if i < other.0.len() && other.0[i].0 < b {
                sb.add_deletion(max(last, other.0[i].0) + y - last, b + y - last);
            }
            while i < other.0.len() && other.0[i].1 < e {
                i += 1;
            }
            y += b - last;
            last = e;
        }
        if i < other.0.len() && other.0[i].0 < last {
            sb.add_deletion(y, other.0[i].1 + y - last);
            i += 1;
        }
        for &(b, e) in &other.0[i..] {
            sb.add_deletion(b + y - last, e + y - last);
        }
        sb.build()
    }

    /// Return an iterator over ranges retained from the source sequence. These will
    /// often be easier to work with than the raw deletions in the representation.
    pub fn range_iter(&self, base_len: usize) -> RangeIter {
        RangeIter {
            deletions: &self.0,
            base_len: base_len,
            i: 0,
            last: 0,
        }
    }
}

pub struct RangeIter<'a> {
    deletions: &'a [(usize, usize)],
    base_len: usize,
    i: usize,
    last: usize,
}

impl<'a> Iterator for RangeIter<'a> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        loop {
            if self.i == self.deletions.len() {
                if self.base_len > self.last {
                    let beg = self.last;
                    self.last = self.base_len;
                    return Some((beg, self.base_len));
                } else {
                    return None;
                }
            } else {
                let beg = self.last;
                let (end, last) = self.deletions[self.i];
                self.last = last;
                self.i += 1;
                if end > beg {
                    return Some((beg, end));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use subset::{Subset, SubsetBuilder};

    const TEST_STR: &'static str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    fn mk_subset(substr: &str, s: &str) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut j = 0;
        for i in 0..s.len() {
            if j < substr.len() && substr.as_bytes()[j] == s.as_bytes()[i] {
                j += 1;
            } else {
                sb.add_deletion(i, i + 1);
            }
        }
        sb.build()
    }

    #[test]
    fn test_apply() {
        let mut sb = SubsetBuilder::new();
        for &(b, e) in &[(0, 1), (2, 4), (6, 11), (13, 14), (15, 18), (19, 23), (24, 26), (31, 32),
                (33, 35), (36, 37), (40, 44), (45, 48), (49, 51), (52, 57), (58, 59)] {
            sb.add_deletion(b, e);
        }
        let s = sb.build();
        assert_eq!("145BCEINQRSTUWZbcdimpvxyz", s.apply_to_string(TEST_STR));
    }

    #[test]
    fn trivial() {
        let s = SubsetBuilder::new().build();
        assert!(s.is_trivial());
    }

    #[test]
    fn test_mk_subset() {
        let substr = "015ABDFHJOPQVYdfgloprsuvz";
        let s = mk_subset(substr, TEST_STR);
        assert_eq!(substr, s.apply_to_string(TEST_STR));
        assert!(!s.is_trivial())
    }

    #[test]
    fn intersect() {
        let s1 = mk_subset("024AEGHJKNQTUWXYZabcfgikqrvy", TEST_STR);
        let s2 = mk_subset("14589DEFGIKMOPQRUXZabcdefglnpsuxyz", TEST_STR);
        assert_eq!("4EGKQUXZabcfgy", s1.intersect(&s2).apply_to_string(TEST_STR));
    }

    fn transform_case(str1: &str, str2: &str, result: &str) {
        let s1 = mk_subset(str1, TEST_STR);
        let s2 = mk_subset(str2, str1);
        let s3 = s2.transform_expand(&s1);
        let str3 = s3.apply_to_string(TEST_STR);
        assert_eq!(result, str3);
        assert_eq!(str2, s3.transform_shrink(&s1).apply_to_string(&str3));
        assert_eq!(str2, s2.transform_intersect(&s1).apply_to_string(TEST_STR));
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
