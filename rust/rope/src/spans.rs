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

//! A module for representing spans (in an interval tree), useful for rich text
//! annotations. It is parameterized over a data type, so can be used for
//! storing different annotations.

use std::{fmt, marker::PhantomData, mem, ops::Range};

use crate::{
    delta::{Delta, DeltaElement, Transformer},
    interval::{Interval, IntervalBounds},
    tree::{Cursor, Leaf, Node, NodeInfo, TreeBuilder},
};

const MIN_LEAF: usize = 32;
const MAX_LEAF: usize = 64;

pub type Spans<T> = Node<SpansInfo<T>>;

#[derive(Clone)]
pub struct SpansInfo<T> {
    n_spans: usize,
    iv: Range<usize>,
    phantom: PhantomData<T>,
}

#[derive(Clone)]
pub struct Span<T: Clone> {
    pub iv: Range<usize>,
    pub data: T,
}

impl<T: Clone> Span<T> {
    pub fn new(iv: Range<usize>, data: T) -> Self {
        Self { iv, data }
    }
}

#[derive(Clone)]
pub struct SpansLeaf<T: Clone> {
    len: usize, // measured in base units
    spans: Vec<Span<T>>,
}

// It would be preferable to derive Default.
// This would however require T to implement Default due to an issue in Rust.
// See: https://github.com/rust-lang/rust/issues/26925
impl<T: Clone> Default for SpansLeaf<T> {
    fn default() -> Self {
        SpansLeaf { len: 0, spans: vec![] }
    }
}

impl<T: Clone> Leaf for SpansLeaf<T> {
    fn len(&self) -> usize {
        self.len
    }

    fn is_ok_child(&self) -> bool {
        self.spans.len() >= MIN_LEAF
    }

    fn push_maybe_split(&mut self, other: &Self, iv: Range<usize>) -> Option<Self> {
        for span in &other.spans {
            let span_iv = span.iv.intersect(&iv).translate_neg(iv.start).translate(self.len);

            if !span_iv.is_empty() {
                self.spans.push(Span { iv: span_iv, data: span.data.clone() });
            }
        }
        self.len += iv.size();

        if self.spans.len() <= MAX_LEAF {
            None
        } else {
            let splitpoint = self.spans.len() / 2; // number of spans
            let splitpoint_units = self.spans[splitpoint].iv.start;
            let mut new = self.spans.split_off(splitpoint);
            for span in &mut new {
                span.iv = span.iv.translate_neg(splitpoint_units);
            }
            let new_len = self.len - splitpoint_units;
            self.len = splitpoint_units;
            Some(SpansLeaf { len: new_len, spans: new })
        }
    }
}

impl<T: Clone> NodeInfo for SpansInfo<T> {
    type L = SpansLeaf<T>;

    fn accumulate(&mut self, other: &Self) {
        self.n_spans += other.n_spans;
        self.iv = self.iv.union(&other.iv);
    }

    fn compute_info(l: &SpansLeaf<T>) -> Self {
        let mut iv = 0..0;
        for span in &l.spans {
            iv = iv.union(&span.iv)
        }
        SpansInfo { n_spans: l.spans.len(), iv, phantom: PhantomData }
    }
}

pub struct SpansBuilder<T: Clone> {
    b: TreeBuilder<SpansInfo<T>>,
    leaf: SpansLeaf<T>,
    len: usize,
    total_len: usize,
}

impl<T: Clone> SpansBuilder<T> {
    pub fn new(total_len: usize) -> Self {
        SpansBuilder { b: TreeBuilder::new(), leaf: SpansLeaf::default(), len: 0, total_len }
    }

    // Precondition: spans must be added in nondecreasing start order.
    fn add_span<IV: IntervalBounds>(&mut self, iv: IV, data: T) {
        let iv = iv.into_interval(self.total_len);
        if self.leaf.spans.len() == MAX_LEAF {
            let mut leaf = mem::take(&mut self.leaf);
            leaf.len = iv.start - self.len;
            self.len = iv.start;
            self.b.push(Node::from_leaf(leaf));
        }
        self.leaf.spans.push(Span { iv: iv.translate_neg(self.len), data })
    }

    pub fn add(&mut self, span: Span<T>) {
        self.add_span(span.iv, span.data)
    }

    // Would make slightly more implementation sense to take total_len as an argument
    // here, but that's not quite the usual builder pattern.
    pub fn build(mut self) -> Spans<T> {
        self.leaf.len = self.total_len - self.len;
        self.b.push(Node::from_leaf(self.leaf));
        self.b.build()
    }
}

pub struct SpanIter<'a, T: Clone> {
    cursor: Cursor<'a, SpansInfo<T>>,
    ix: usize,
}

impl<T: Clone> Spans<T> {
    /// Perform operational transformation on a spans object intended to be edited into
    /// a sequence at the given offset.

    // Note: this implementation is not efficient for very large Spans objects, as it
    // traverses all spans linearly. A more sophisticated approach would be to traverse
    // the tree, and only delve into subtrees that are transformed.
    pub fn transform<N: NodeInfo>(
        &self,
        base_start: usize,
        base_end: usize,
        xform: &mut Transformer<'_, N>,
    ) -> Self {
        // TODO: maybe should take base as an Interval and figure out "after" from that
        let new_start = xform.transform(base_start, false);
        let new_end = xform.transform(base_end, true);
        let mut builder = SpansBuilder::new(new_end - new_start);
        for span in self.iter() {
            let start = xform.transform(span.iv.start + base_start, false) - new_start;
            let end = xform.transform(span.iv.end + base_start, false) - new_start;
            if start < end {
                let span = Span { iv: start..end, data: span.data };
                // TODO: could imagine using a move iterator and avoiding clone, but it's not easy.
                builder.add(span);
            }
        }
        builder.build()
    }

    /// Creates a new Spans instance by merging spans from `other` with `self`,
    /// using a closure to transform values.
    ///
    /// New spans are created from non-overlapping regions of existing spans,
    /// and by combining overlapping regions into new spans. In all cases,
    /// new values are generated by calling a closure that transforms the
    /// value of the existing span or spans.
    ///
    /// # Panics
    ///
    /// Panics if `self` and `other` have different lengths.
    pub fn merge<F, O>(&self, other: &Self, mut f: F) -> Spans<O>
    where
        F: FnMut(T, Option<T>) -> O,
        O: Clone,
    {
        //TODO: confirm that this is sensible behaviour
        assert_eq!(self.len(), other.len());
        let mut sb = SpansBuilder::new(self.len());

        // red/blue is just a better name than one/two or me/other
        let mut iter_red = self.iter();
        let mut iter_blue = other.iter();

        let mut next_red = iter_red.next();
        let mut next_blue = iter_blue.next();

        loop {
            // exit conditions:
            if next_red.is_none() && next_blue.is_none() {
                // all merged.
                break;
            } else if next_red.is_none() != next_blue.is_none() {
                // one side is exhausted; append remaining items from other side.
                let iter = if next_red.is_some() { iter_red } else { iter_blue };
                // add this item
                let span = next_red.or(next_blue).unwrap();
                let span = Span { iv: span.iv, data: f(span.data, None) };
                sb.add(span);

                for span in iter {
                    let span = Span { iv: span.iv, data: f(span.data, None) };
                    sb.add(span);
                }
                break;
            }

            // body:
            let Span { iv: mut red_iv, data: red_val } = next_red.clone().unwrap();
            let Span { iv: mut blue_iv, data: blue_val } = next_blue.clone().unwrap();

            if red_iv.intersect(&blue_iv).is_empty() {
                // spans do not overlap. Add the leading span & advance that iter.
                if red_iv.is_before(blue_iv.start) {
                    let span = Span::new(red_iv.clone(), f(red_val.clone(), None));
                    sb.add(span);
                    next_red = iter_red.next();
                } else {
                    let span = Span::new(blue_iv.clone(), f(blue_val.clone(), None));
                    sb.add(span);
                    next_blue = iter_blue.next();
                }
                continue;
            }
            assert!(!red_iv.intersect(&blue_iv).is_empty());

            // if these two spans do not share a start point, create a new span from
            // the prefix of the leading span.
            use std::cmp::Ordering;

            match red_iv.start.cmp(&blue_iv.start) {
                Ordering::Less => {
                    let iv = red_iv.prefix(&blue_iv);
                    let span = Span::new(iv.clone(), f(red_val.clone(), None));
                    sb.add(span);
                    red_iv = red_iv.suffix(&iv);
                }
                Ordering::Greater => {
                    let iv = blue_iv.prefix(&red_iv);
                    let span = Span::new(iv.clone(), f(blue_val.clone(), None));
                    sb.add(span);
                    blue_iv = blue_iv.suffix(&iv);
                }
                Ordering::Equal => {}
            }

            assert!(red_iv.start == blue_iv.start);
            // create a new span by merging the overlapping regions.
            let iv = red_iv.intersect(&blue_iv);
            assert!(!iv.is_empty());
            let span = Span::new(iv.clone(), f(red_val.clone(), Some(blue_val.clone())));
            sb.add(span);

            // if an old span was consumed by this new span, advance
            // else reuse remaining span (set next_red/blue) for the next loop iteration
            red_iv = red_iv.suffix(&iv);
            blue_iv = blue_iv.suffix(&iv);
            assert!(red_iv.is_empty() || blue_iv.is_empty());

            if red_iv.is_empty() {
                next_red = iter_red.next();
            } else {
                let red_span = Span { iv: red_iv, data: red_val };
                next_red = Some(red_span);
            }

            if blue_iv.is_empty() {
                next_blue = iter_blue.next();
            } else {
                let blue_span = Span { iv: blue_iv, data: blue_val };
                next_blue = Some(blue_span);
            }
        }
        sb.build()
    }

    // possible future: an iterator that takes an interval, so results are the same as
    // taking a subseq on the spans object. Would require specialized Cursor.
    pub fn iter(&self) -> SpanIter<'_, T> {
        SpanIter { cursor: Cursor::new(self, 0), ix: 0 }
    }

    /// Applies a generic delta to `self`, inserting empty spans for any
    /// added regions.
    ///
    /// This is intended to be used to keep spans up to date with a `Rope`
    /// as edits occur.
    pub fn apply_shape<M: NodeInfo>(&mut self, delta: &Delta<M>) {
        let mut b = TreeBuilder::new();
        for elem in &delta.els {
            match elem {
                DeltaElement::Copy(range) => b.push(self.subseq(range.clone())),
                DeltaElement::Insert(n) => b.push(SpansBuilder::new(n.len()).build()),
            }
        }
        *self = b.build();
    }

    /// Deletes all spans that intersect with `interval` and that come after.
    pub fn delete_after(&mut self, interval: Range<usize>) {
        let mut builder = SpansBuilder::new(self.len());

        for span in self.iter() {
            // check if spans overlaps with interval
            if span.iv.intersect(&interval).is_empty() {
                // keep the ones that are not overlapping
                builder.add(span);
            } else {
                // all remaining spans are invalid
                break;
            }
        }
        *self = builder.build();
    }
}

impl<T: Clone + fmt::Debug> fmt::Debug for Spans<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let strs = self
            .iter()
            .map(|span| format!("{:?}: {:?}", span.iv, span.data))
            .collect::<Vec<String>>();
        write!(f, "len: {}\nspans:\n\t{}", self.len(), &strs.join("\n\t"))
    }
}

impl<'a, T: Clone> Iterator for SpanIter<'a, T> {
    type Item = Span<T>;

    fn next(&mut self) -> Option<Span<T>> {
        if let Some((leaf, start_pos)) = self.cursor.get_leaf() {
            if leaf.spans.is_empty() {
                return None;
            }
            let leaf_start = self.cursor.pos() - start_pos;
            let span = &leaf.spans[self.ix];
            self.ix += 1;
            if self.ix == leaf.spans.len() {
                let _ = self.cursor.next_leaf();
                self.ix = 0;
            }
            let span = Span { iv: span.iv.translate(leaf_start), data: span.data.clone() };
            return Some(span);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_merge() {
        // merging 1 1 1 1 1 1 1 1 1 16
        // with    2 2 4 4     8 8
        // ==      3 3 5 5 1 1 9 9 1 16
        let mut sb = SpansBuilder::new(10);
        sb.add(Span::new(0..9, 1u32));
        sb.add(Span::new(9..10, 16));
        let red = sb.build();

        let mut sb = SpansBuilder::new(10);
        sb.add(Span::new(0..2, 2));
        sb.add(Span::new(2..4, 4));
        sb.add(Span::new(6..8, 8));
        let blue = sb.build();

        let merged = red.merge(&blue, |r, b| b.map(|b| b + r).unwrap_or(r));

        let mut merged_iter = merged.iter();
        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (0..2));
        assert_eq!(span.data, 3);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (2..4));
        assert_eq!(span.data, 5);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (4..6));
        assert_eq!(span.data, 1);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (6..8));
        assert_eq!(span.data, 9);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (8..9));
        assert_eq!(span.data, 1);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (9..10));
        assert_eq!(span.data, 16);

        assert!(merged_iter.next().is_none());
    }

    #[test]
    fn test_merge_2() {
        // 1 1 1   4 4
        //   2 2 2 2     8 9
        let mut sb = SpansBuilder::new(9);
        sb.add(Span::new(0..3, 1));
        sb.add(Span::new(4..6, 4));
        let blue = sb.build();

        let mut sb = SpansBuilder::new(9);
        sb.add(Span::new(1..5, 2));
        sb.add(Span::new(7..8, 8));
        sb.add(Span::new(8..9, 9));
        let red = sb.build();

        let merged = red.merge(&blue, |r, b| b.map(|b| b + r).unwrap_or(r));

        let mut merged_iter = merged.iter();
        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (0..1));
        assert_eq!(span.data, 1);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (1..3));
        assert_eq!(span.data, 3);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (3..4));
        assert_eq!(span.data, 2);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (4..5));
        assert_eq!(span.data, 6);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (5..6));
        assert_eq!(span.data, 4);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (7..8));
        assert_eq!(span.data, 8);

        let span = merged_iter.next().unwrap();
        assert_eq!(span.iv, (8..9));
        assert_eq!(span.data, 9);

        assert!(merged_iter.next().is_none());
    }

    #[test]
    fn test_delete_after() {
        let mut sb = SpansBuilder::new(11);
        sb.add(Span::new(1..2, 2));
        sb.add(Span::new(3..5, 8));
        sb.add(Span::new(6..8, 9));
        sb.add(Span::new(9..10, 1));
        sb.add(Span::new(10..11, 1));
        let mut spans = sb.build();

        spans.delete_after(4..7);

        assert_eq!(spans.iter().count(), 1);

        let span = spans.iter().next().unwrap();
        assert_eq!(span.iv, (1..2));
        assert_eq!(span.data, 2);
    }

    #[test]
    fn delete_after_big_at_start() {
        let mut sb = SpansBuilder::new(10);
        sb.add(Span::new(0..10, 0));
        let mut spans = sb.build();
        assert_eq!(spans.iter().count(), 1);

        spans.delete_after(1..2);
        assert_eq!(spans.iter().count(), 0);
    }

    #[test]
    fn delete_after_big_and_small() {
        let mut sb = SpansBuilder::new(10);
        sb.add(Span::new(0..10, 0));
        sb.add(Span::new(3..10, 1));
        let mut spans = sb.build();
        assert_eq!(spans.iter().count(), 2);

        spans.delete_after(1..2);
        assert_eq!(spans.iter().count(), 0);
    }

    #[test]
    fn delete_after_empty() {
        let mut sb = SpansBuilder::new(10);
        sb.add(Span::new(0..3, 0));

        let mut spans = sb.build();
        assert_eq!(spans.iter().count(), 1);

        spans.delete_after(5..7);
        assert_eq!(spans.iter().count(), 1);
    }
}
