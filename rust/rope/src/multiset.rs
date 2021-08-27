// Copyright 2017 The xi-editor Authors.
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

//! A data structure for representing multi-subsets of sequences (typically strings).

use std::cmp;

// These two imports are for the `apply` method only.
use crate::interval::Interval;
use crate::tree::{Node, NodeInfo, TreeBuilder};
use std::fmt;
use std::slice;

#[derive(Clone, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
struct Segment {
    len: usize,
    count: usize,
}

/// Represents a multi-subset of a string, that is a subset where elements can
/// be included multiple times. This is represented as each element of the
/// string having a "count" which is the number of times that element is
/// included in the set.
///
/// Internally, this is stored as a list of "segments" with a length and a count.
#[derive(Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Subset {
    /// Invariant, maintained by `SubsetBuilder`: all `Segment`s have non-zero
    /// length, and no `Segment` has the same count as the one before it.
    segments: Vec<Segment>,
}

#[derive(Default)]
pub struct SubsetBuilder {
    segments: Vec<Segment>,
    total_len: usize,
}

impl SubsetBuilder {
    pub fn new() -> SubsetBuilder {
        SubsetBuilder::default()
    }

    /// Intended for use with `add_range` to ensure the total length of the
    /// `Subset` corresponds to the document length.
    pub fn pad_to_len(&mut self, total_len: usize) {
        if total_len > self.total_len {
            let cur_len = self.total_len;
            self.push_segment(total_len - cur_len, 0);
        }
    }

    /// Sets the count for a given range. This method must be called with a
    /// non-empty range with `begin` not before the largest range or segment added
    /// so far. Gaps will be filled with a 0-count segment.
    pub fn add_range(&mut self, begin: usize, end: usize, count: usize) {
        assert!(begin >= self.total_len, "ranges must be added in non-decreasing order");
        // assert!(begin < end, "ranges added must be non-empty: [{},{})", begin, end);
        if begin >= end {
            return;
        }
        let len = end - begin;
        let cur_total_len = self.total_len;

        // add 0-count segment to fill any gap
        if begin > self.total_len {
            self.push_segment(begin - cur_total_len, 0);
        }

        self.push_segment(len, count);
    }

    /// Assign `count` to the next `len` elements in the string.
    /// Will panic if called with `len==0`.
    pub fn push_segment(&mut self, len: usize, count: usize) {
        assert!(len > 0, "can't push empty segment");
        self.total_len += len;

        // merge into previous segment if possible
        if let Some(last) = self.segments.last_mut() {
            if last.count == count {
                last.len += len;
                return;
            }
        }

        self.segments.push(Segment { len, count });
    }

    pub fn build(self) -> Subset {
        Subset { segments: self.segments }
    }
}

/// Determines which elements of a `Subset` a method applies to
/// based on the count of the element.
#[derive(Clone, Copy, Debug)]
pub enum CountMatcher {
    Zero,
    NonZero,
    All,
}

impl CountMatcher {
    fn matches(self, seg: &Segment) -> bool {
        match self {
            CountMatcher::Zero => (seg.count == 0),
            CountMatcher::NonZero => (seg.count != 0),
            CountMatcher::All => true,
        }
    }
}

impl Subset {
    /// Creates an empty `Subset` of a string of length `len`
    pub fn new(len: usize) -> Subset {
        let mut sb = SubsetBuilder::new();
        sb.pad_to_len(len);
        sb.build()
    }

    /// Mostly for testing.
    pub fn delete_from_string(&self, s: &str) -> String {
        let mut result = String::new();
        for (b, e) in self.range_iter(CountMatcher::Zero) {
            result.push_str(&s[b..e]);
        }
        result
    }

    // Maybe Subset should be a pure data structure and this method should
    // be a method of Node.
    /// Builds a version of `s` with all the elements in this `Subset` deleted from it.
    pub fn delete_from<N: NodeInfo>(&self, s: &Node<N>) -> Node<N> {
        let mut b = TreeBuilder::new();
        for (beg, end) in self.range_iter(CountMatcher::Zero) {
            b.push_slice(s, Interval::new(beg, end));
        }
        b.build()
    }

    /// The length of the resulting sequence after deleting this subset. A
    /// convenience alias for `self.count(CountMatcher::Zero)` to reduce
    /// thinking about what that means in the cases where the length after
    /// delete is what you want to know.
    ///
    /// `self.delete_from_string(s).len() = self.len(s.len())`
    pub fn len_after_delete(&self) -> usize {
        self.count(CountMatcher::Zero)
    }

    /// Count the total length of all the segments matching `matcher`.
    pub fn count(&self, matcher: CountMatcher) -> usize {
        self.segments.iter().filter(|seg| matcher.matches(seg)).map(|seg| seg.len).sum()
    }

    /// Convenience alias for `self.count(CountMatcher::All)`
    pub fn len(&self) -> usize {
        self.count(CountMatcher::All)
    }

    /// Determine whether the subset is empty.
    /// In this case deleting it would do nothing.
    pub fn is_empty(&self) -> bool {
        (self.segments.is_empty()) || ((self.segments.len() == 1) && (self.segments[0].count == 0))
    }

    /// Compute the union of two subsets. The count of an element in the
    /// result is the sum of the counts in the inputs.
    pub fn union(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        for zseg in self.zip(other) {
            sb.push_segment(zseg.len, zseg.a_count + zseg.b_count);
        }
        sb.build()
    }

    /// Compute the difference of two subsets. The count of an element in the
    /// result is the subtraction of the counts of other from self.
    pub fn subtract(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        for zseg in self.zip(other) {
            assert!(
                zseg.a_count >= zseg.b_count,
                "can't subtract {} from {}",
                zseg.a_count,
                zseg.b_count
            );
            sb.push_segment(zseg.len, zseg.a_count - zseg.b_count);
        }
        sb.build()
    }

    /// Compute the bitwise xor of two subsets, useful as a reversible
    /// difference. The count of an element in the result is the bitwise xor
    /// of the counts of the inputs. Unchanged segments will be 0.
    ///
    /// This works like set symmetric difference when all counts are 0 or 1
    /// but it extends nicely to the case of larger counts.
    pub fn bitxor(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        for zseg in self.zip(other) {
            sb.push_segment(zseg.len, zseg.a_count ^ zseg.b_count);
        }
        sb.build()
    }

    /// Map the contents of `self` into the 0-regions of `other`.
    /// Precondition: `self.count(CountMatcher::All) == other.count(CountMatcher::Zero)`
    fn transform(&self, other: &Subset, union: bool) -> Subset {
        let mut sb = SubsetBuilder::new();
        let mut seg_iter = self.segments.iter();
        let mut cur_seg = Segment { len: 0, count: 0 };
        for oseg in &other.segments {
            if oseg.count > 0 {
                sb.push_segment(oseg.len, if union { oseg.count } else { 0 });
            } else {
                // fill 0-region with segments from self.
                let mut to_be_consumed = oseg.len;
                while to_be_consumed > 0 {
                    if cur_seg.len == 0 {
                        cur_seg = seg_iter
                            .next()
                            .expect("self must cover all 0-regions of other")
                            .clone();
                    }
                    // consume as much of the segment as possible and necessary
                    let to_consume = cmp::min(cur_seg.len, to_be_consumed);
                    sb.push_segment(to_consume, cur_seg.count);
                    to_be_consumed -= to_consume;
                    cur_seg.len -= to_consume;
                }
            }
        }
        assert_eq!(cur_seg.len, 0, "the 0-regions of other must be the size of self");
        assert_eq!(seg_iter.next(), None, "the 0-regions of other must be the size of self");
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
        self.transform(other, false)
    }

    /// The same as taking transform_expand and then unioning with `other`.
    pub fn transform_union(&self, other: &Subset) -> Subset {
        self.transform(other, true)
    }

    /// Transform subset through other coordinate transform, shrinking.
    /// The following equation is satisfied:
    ///
    /// C = A.transform_expand(B)
    ///
    /// B.transform_shrink(C).delete_from_string(C.delete_from_string(s)) =
    ///   A.delete_from_string(B.delete_from_string(s))
    pub fn transform_shrink(&self, other: &Subset) -> Subset {
        let mut sb = SubsetBuilder::new();
        // discard ZipSegments where the shrinking set has positive count
        for zseg in self.zip(other) {
            // TODO: should this actually do something like subtract counts?
            if zseg.b_count == 0 {
                sb.push_segment(zseg.len, zseg.a_count);
            }
        }
        sb.build()
    }

    /// Return an iterator over the ranges with a count matching the `matcher`.
    /// These will often be easier to work with than raw segments.
    pub fn range_iter(&self, matcher: CountMatcher) -> RangeIter {
        RangeIter { seg_iter: self.segments.iter(), consumed: 0, matcher }
    }

    /// Convenience alias for `self.range_iter(CountMatcher::Zero)`.
    /// Semantically iterates the ranges of the complement of this `Subset`.
    pub fn complement_iter(&self) -> RangeIter {
        self.range_iter(CountMatcher::Zero)
    }

    /// Return an iterator over `ZipSegment`s where each `ZipSegment` contains
    /// the count for both self and other in that range. The two `Subset`s
    /// must have the same total length.
    ///
    /// Each returned `ZipSegment` will differ in at least one count.
    pub fn zip<'a>(&'a self, other: &'a Subset) -> ZipIter<'a> {
        ZipIter {
            a_segs: self.segments.as_slice(),
            b_segs: other.segments.as_slice(),
            a_i: 0,
            b_i: 0,
            a_consumed: 0,
            b_consumed: 0,
            consumed: 0,
        }
    }

    /// Find the complement of this Subset. Every 0-count element will have a
    /// count of 1 and every non-zero element will have a count of 0.
    pub fn complement(&self) -> Subset {
        let mut sb = SubsetBuilder::new();
        for seg in &self.segments {
            if seg.count == 0 {
                sb.push_segment(seg.len, 1);
            } else {
                sb.push_segment(seg.len, 0);
            }
        }
        sb.build()
    }

    /// Return a `Mapper` that can be use to map coordinates in the document to coordinates
    /// in this `Subset`, but only in non-decreasing order for performance reasons.
    pub fn mapper(&self, matcher: CountMatcher) -> Mapper {
        Mapper {
            range_iter: self.range_iter(matcher),
            last_i: 0, // indices only need to be in non-decreasing order, not increasing
            cur_range: (0, 0), // will immediately try to consume next range
            subset_amount_consumed: 0,
        }
    }
}

impl fmt::Debug for Subset {
    /// Use the alternate flag (`#`) to print a more compact representation
    /// where each character represents the count of one element:
    /// '-' is 0, '#' is 1, 2-9 are digits, `+` is >9
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if f.alternate() {
            for s in &self.segments {
                let chr = if s.count == 0 {
                    '-'
                } else if s.count == 1 {
                    '#'
                } else if s.count <= 9 {
                    ((s.count as u8) + b'0') as char
                } else {
                    '+'
                };
                for _ in 0..s.len {
                    write!(f, "{}", chr)?;
                }
            }
            Ok(())
        } else {
            f.debug_tuple("Subset").field(&self.segments).finish()
        }
    }
}

pub struct RangeIter<'a> {
    seg_iter: slice::Iter<'a, Segment>,
    pub consumed: usize,
    matcher: CountMatcher,
}

impl<'a> Iterator for RangeIter<'a> {
    type Item = (usize, usize);

    fn next(&mut self) -> Option<(usize, usize)> {
        for seg in &mut self.seg_iter {
            self.consumed += seg.len;
            if self.matcher.matches(seg) {
                return Some((self.consumed - seg.len, self.consumed));
            }
        }
        None
    }
}

/// See `Subset::zip`
pub struct ZipIter<'a> {
    a_segs: &'a [Segment],
    b_segs: &'a [Segment],
    a_i: usize,
    b_i: usize,
    a_consumed: usize,
    b_consumed: usize,
    pub consumed: usize,
}

/// See `Subset::zip`
#[derive(Clone, Debug)]
pub struct ZipSegment {
    len: usize,
    a_count: usize,
    b_count: usize,
}

impl<'a> Iterator for ZipIter<'a> {
    type Item = ZipSegment;

    /// Consume as far as possible from `self.consumed` until reaching a
    /// segment boundary in either `Subset`, and return the resulting
    /// `ZipSegment`. Will panic if it reaches the end of one `Subset` before
    /// the other, that is when they have different total length.
    fn next(&mut self) -> Option<ZipSegment> {
        match (self.a_segs.get(self.a_i), self.b_segs.get(self.b_i)) {
            (None, None) => None,
            (None, Some(_)) | (Some(_), None) => {
                panic!("can't zip Subsets of different base lengths.")
            }
            (
                Some(&Segment { len: a_len, count: a_count }),
                Some(&Segment { len: b_len, count: b_count }),
            ) => {
                let len = match (a_len + self.a_consumed).cmp(&(b_len + self.b_consumed)) {
                    cmp::Ordering::Equal => {
                        self.a_consumed += a_len;
                        self.a_i += 1;
                        self.b_consumed += b_len;
                        self.b_i += 1;
                        self.a_consumed - self.consumed
                    }
                    cmp::Ordering::Less => {
                        self.a_consumed += a_len;
                        self.a_i += 1;
                        self.a_consumed - self.consumed
                    }
                    cmp::Ordering::Greater => {
                        self.b_consumed += b_len;
                        self.b_i += 1;
                        self.b_consumed - self.consumed
                    }
                };
                self.consumed += len;
                Some(ZipSegment { len, a_count, b_count })
            }
        }
    }
}

pub struct Mapper<'a> {
    range_iter: RangeIter<'a>,
    // Not actually necessary for computation, just for dynamic checking of invariant
    last_i: usize,
    cur_range: (usize, usize),
    pub subset_amount_consumed: usize,
}

impl<'a> Mapper<'a> {
    /// Map a coordinate in the document this subset corresponds to, to a
    /// coordinate in the subset matched by the `CountMatcher`. For example,
    /// if the Subset is a set of deletions and the matcher is
    /// `CountMatcher::NonZero`, this would map indices in the union string to
    /// indices in the tombstones string.
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
    pub fn doc_index_to_subset(&mut self, i: usize) -> usize {
        assert!(
            i >= self.last_i,
            "method must be called with i in non-decreasing order. i={}<{}=last_i",
            i,
            self.last_i
        );
        self.last_i = i;

        while i >= self.cur_range.1 {
            self.subset_amount_consumed += self.cur_range.1 - self.cur_range.0;
            self.cur_range = match self.range_iter.next() {
                Some(range) => range,
                // past the end of the subset
                None => {
                    // ensure we don't try to consume any more
                    self.cur_range = (usize::max_value(), usize::max_value());
                    return self.subset_amount_consumed;
                }
            }
        }

        if i >= self.cur_range.0 {
            let dist_in_range = i - self.cur_range.0;
            dist_in_range + self.subset_amount_consumed
        } else {
            // not in the subset
            self.subset_amount_consumed
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::multiset::*;
    use crate::test_helpers::find_deletions;

    const TEST_STR: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

    #[test]
    fn test_apply() {
        let mut sb = SubsetBuilder::new();
        for &(b, e) in &[
            (0, 1),
            (2, 4),
            (6, 11),
            (13, 14),
            (15, 18),
            (19, 23),
            (24, 26),
            (31, 32),
            (33, 35),
            (36, 37),
            (40, 44),
            (45, 48),
            (49, 51),
            (52, 57),
            (58, 59),
        ] {
            sb.add_range(b, e, 1);
        }
        sb.pad_to_len(TEST_STR.len());
        let s = sb.build();
        println!("{:?}", s);
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
        let c = s.complement();
        // deleting the complement of the deletions we found should yield the deletions
        assert_eq!("123ABCabcxyz", c.delete_from_string(TEST_STR));
    }

    #[test]
    fn test_mapper() {
        let substr = "469ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwz";
        let s = find_deletions(substr, TEST_STR);
        let mut m = s.mapper(CountMatcher::NonZero);
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
        let mut m = s.mapper(CountMatcher::NonZero);
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
        assert_eq!(str2, s1.transform_shrink(&s3).delete_from_string(&str3));
        assert_eq!(str2, s2.transform_union(&s1).delete_from_string(TEST_STR));
    }

    #[test]
    fn transform() {
        transform_case(
            "02345678BCDFGHKLNOPQRTUVXZbcefghjlmnopqrstwx",
            "027CDGKLOTUbcegopqrw",
            "01279ACDEGIJKLMOSTUWYabcdegikopqruvwyz",
        );
        transform_case(
            "01234678DHIKLMNOPQRUWZbcdhjostvy",
            "136KLPQZvy",
            "13569ABCEFGJKLPQSTVXYZaefgiklmnpqruvwxyz",
        );
        transform_case(
            "0125789BDEFIJKLMNPVXabdjmrstuwy",
            "12BIJVXjmrstu",
            "12346ABCGHIJOQRSTUVWXYZcefghijklmnopqrstuvxz",
        );
        transform_case(
            "12456789ABCEFGJKLMNPQRSTUVXYadefghkrtwxz",
            "15ACEFGKLPRUVYdhrtx",
            "0135ACDEFGHIKLOPRUVWYZbcdhijlmnopqrstuvxy",
        );
        transform_case(
            "0128ABCDEFGIJMNOPQXYZabcfgijkloqruvy",
            "2CEFGMZabijloruvy",
            "2345679CEFGHKLMRSTUVWZabdehijlmnoprstuvwxyz",
        );
        transform_case(
            "01245689ABCDGJKLMPQSTWXYbcdfgjlmnosvy",
            "01245ABCDJLQSWXYgsv",
            "0123457ABCDEFHIJLNOQRSUVWXYZaeghikpqrstuvwxz",
        );
    }
}
