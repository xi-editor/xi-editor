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

//! A module for representing spans (in an interval tree), useful for rich text
//! annotations. It is parameterized over a data type, so can be used for
//! storing different annotations.

use std::marker::PhantomData;
use std::mem;

use tree::{Leaf, Node, NodeInfo, TreeBuilder, Cursor};
use delta::Transformer;
use interval::Interval;

const MIN_LEAF: usize = 32;
const MAX_LEAF: usize = 64;

pub type Spans<T> = Node<SpansInfo<T>>;

#[derive(Clone)]
pub struct Span<T: Clone> {
    iv: Interval,
    data: T,
}

#[derive(Clone, Default)]
pub struct SpansLeaf<T: Clone> {
    len: usize,  // measured in base units
    spans: Vec<Span<T>>,
}

#[derive(Clone)]
pub struct SpansInfo<T> {
    n_spans: usize,
    iv: Interval,
    phantom: PhantomData<T>,
}

impl<T: Clone + Default> Leaf for SpansLeaf<T> {
    fn len(&self) -> usize {
        self.len
    }

    fn is_ok_child(&self) -> bool {
        self.spans.len() >= MIN_LEAF
    }

    fn push_maybe_split(&mut self, other: &Self, iv: Interval) -> Option<Self> {
        let iv_start = iv.start();
        for span in &other.spans {
            let span_iv = span.iv.intersect(iv).translate_neg(iv_start).translate(self.len);
            if !span_iv.is_empty() {
                self.spans.push(Span {
                    iv: span_iv,
                    data: span.data.clone(),
                });
            }
        }
        self.len += iv.size();

        if self.spans.len() <= MAX_LEAF {
            None
        } else {
            let splitpoint = self.spans.len() / 2;  // number of spans
            let splitpoint_units = self.spans[splitpoint].iv.start();
            let mut new = self.spans.split_off(splitpoint);
            for span in &mut new {
                span.iv = span.iv.translate_neg(splitpoint_units);
            }
            let new_len = self.len - splitpoint_units;
            self.len = splitpoint_units;
            Some(SpansLeaf {
                len: new_len,
                spans: new,
            })
        }
    }
}

impl<T: Clone + Default> NodeInfo for SpansInfo<T> {
    type L = SpansLeaf<T>;

    fn accumulate(&mut self, other: &Self) {
        self.n_spans += other.n_spans;
        self.iv = self.iv.union(other.iv);
    }

    fn compute_info(l: &SpansLeaf<T>) -> Self {
        let mut iv = Interval::new_closed_open(0, 0);  // should be Interval::default?
        for span in &l.spans {
            iv = iv.union(span.iv);
        }
        SpansInfo {
            n_spans: l.spans.len(),
            iv: iv,
            phantom: PhantomData,
        }
    }
}

pub struct SpansBuilder<T: Clone + Default> {
    b: TreeBuilder<SpansInfo<T>>,
    leaf: SpansLeaf<T>,
    len: usize,
    total_len: usize,
}

impl<T: Clone + Default> SpansBuilder<T> {
    pub fn new(total_len: usize) -> Self {
        SpansBuilder {
            b: TreeBuilder::new(),
            leaf: SpansLeaf::default(),
            len: 0,
            total_len: total_len,
        }
    }

    // Precondition: spans must be added in nondecreasing start order.
    // Maybe take Span struct instead of separate iv, data args?
    pub fn add_span(&mut self, iv: Interval, data: T) {
        if self.leaf.spans.len() == MAX_LEAF {
            let mut leaf = mem::replace(&mut self.leaf, SpansLeaf::default());
            leaf.len = iv.start() - self.len;
            self.len = iv.start();
            self.b.push(Node::from_leaf(leaf));
        }
        self.leaf.spans.push(Span {
            iv: iv.translate_neg(self.len),
            data: data,
        })
    }

    // Would make slightly more implementation sense to take total_len as an argument
    // here, but that's not quite the usual builder pattern.
    pub fn build(mut self) -> Spans<T> {
        self.leaf.len = self.total_len - self.len;
        self.b.push(Node::from_leaf(self.leaf));
        self.b.build()
    }
}

pub struct SpanIter<'a, T: 'a + Clone + Default> {
    cursor: Cursor<'a, SpansInfo<T>>,
    ix: usize,
}

impl<T: Clone + Default> Spans<T> {
    /// Perform operational transformation on a spans object intended to be edited into
    /// a sequence at the given offset.

    // Note: this implementation is not efficient for very large Spans objects, as it
    // traverses all spans linearly. A more sophisticated approach would be to traverse
    // the tree, and only delve into subtrees that are transformed.
    pub fn transform<N: NodeInfo>(&self, base_start: usize, base_end: usize,
            xform: &mut Transformer<N>) -> Self {
        // TODO: maybe should take base as an Interval and figure out "after" from that
        let new_start = xform.transform(base_start, false);
        let new_end = xform.transform(base_end, true);
        let mut builder = SpansBuilder::new(new_end - new_start);
        for (iv, data) in self.iter() {
            let (start_closed, end_closed) = (iv.is_start_closed(), iv.is_end_closed());
            let start = xform.transform(iv.start() + base_start, !start_closed) - new_start;
            let end = xform.transform(iv.end() + base_start, end_closed) - new_start;
            if start < end || (start_closed && end_closed) {
                let iv = Interval::new(start, start_closed, end, end_closed);
                // TODO: could imagine using a move iterator and avoiding clone, but it's not easy.
                builder.add_span(iv, data.clone());
            }
        }
        builder.build()
    }

    // possible future: an iterator that takes an interval, so results are the same as
    // taking a subseq on the spans object. Would require specialized Cursor.
    pub fn iter(&self) -> SpanIter<T> {
        SpanIter {
            cursor: Cursor::new(self, 0),
            ix: 0,
        }
    }
}

impl<'a, T: Clone + Default> Iterator for SpanIter<'a, T> {
    type Item = (Interval, &'a T);

    fn next(&mut self) -> Option<(Interval, &'a T)> {
        if let Some((leaf, start_pos)) = self.cursor.get_leaf() {
            if leaf.spans.is_empty() { return None; }
            let leaf_start = self.cursor.pos() - start_pos;
            let span = &leaf.spans[self.ix];
            self.ix += 1;
            if self.ix == leaf.spans.len() {
                let _ = self.cursor.next_leaf();
                self.ix = 0;
            }
            return Some((span.iv.translate(leaf_start), &span.data));
        }
        None
    }
}
