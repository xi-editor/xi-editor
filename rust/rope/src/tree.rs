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

//! A general b-tree structure suitable for ropes and the like.

use std::sync::Arc;
use std::cmp::{min,max};

const MIN_CHILDREN: usize = 4;
const MAX_CHILDREN: usize = 8;

pub trait NodeInfo: Clone {
    fn accumulate(&mut self, other: &Self);

    // default?
}

pub trait Leaf<N>: Sized + Clone + Default {

    // measurement of leaf in base units
    fn len(&self) -> usize;

    // return info
    fn compute_info(&self) -> N;

    // generally a minimum size requirement for leaves
    fn is_ok_child(&self) -> bool;

    // start and end are in "base units"
    // if end == len + 1, then include trailing fragment
    // (note: some leaf types don't have fragments, but still have to
    // deal with len + 1)
    // generally implements a maximum size
    // Invariant: if one or the other input is empty, then no split

    // Invariant: if either input satisfies is_ok_child, then
    // satisfies this, as does the optional split.

    fn push_maybe_split(&mut self, other: &Self, start: usize, end: usize) -> Option<Self>;

    // same meaning as push_maybe_split starting from an empty
    // leaf, but maybe can be implemented more efficiently?
    // TODO: remove if it doesn't pull its weight
    fn subseq(&self, start: usize, end: usize) -> Self {
        let mut result = Self::default();
        if result.push_maybe_split(self, start, end).is_some() {
            panic!("unexpected split");
        }
        result
    }
}

struct Node<N: NodeInfo, L: Leaf<N>>(Arc<NodeBody<N, L>>);

impl<N: NodeInfo, L: Leaf<N>> Clone for Node<N, L> {
    fn clone(&self) -> Self {
        Node(self.0.clone())
    }
}

#[derive(Clone)]
struct NodeBody<N: NodeInfo, L: Leaf<N>> {
    height: usize,
    len: usize,
    info: N,
    val: NodeVal<N, L>,
}

#[derive(Clone)]
enum NodeVal<N: NodeInfo, L: Leaf<N>> {
    Leaf(L),
    Internal(Vec<Node<N, L>>),
}

// stateless only for now, but we can consider other choices

// also consider making Metric a newtype for usize, so type system can
// help separate metrics
pub trait Metric<N: NodeInfo, L: Leaf<N>> {
    // probably want len also
    fn measure(&N) -> usize;

    fn to_base_units(l: &L, in_measured_units: usize) -> usize;

    fn from_base_units(l: &L, in_base_units: usize) -> usize;
}

impl<N: NodeInfo, L: Leaf<N>> Node<N, L> {
    fn from_leaf(l: L) -> Node<N, L> {
        let len = l.len();
        let info = l.compute_info();
        Node(Arc::new(
            NodeBody {
            height: 0,
            len: len,
            info: info,
            val: NodeVal::Leaf(l),
        }))
    }

    fn from_nodes(nodes: Vec<Node<N, L>>) -> Node<N, L> {
        let height = nodes[0].0.height + 1;
        let mut len = nodes[0].0.len;
        let mut info = nodes[0].0.info.clone();
        for child in &nodes[1..] {
            len += child.0.len;
            info.accumulate(&child.0.info);
        }
        Node(Arc::new(
            NodeBody {
            height: height,
            len: len,
            info: info,
            val: NodeVal::Internal(nodes),
        }))
    }

    fn height(&self) -> usize {
        self.0.height
    }

    fn get_children(&self) -> &[Node<N, L>] {
        if let &NodeVal::Internal(ref v) = &self.0.val {
            v
        } else {
            panic!("get_children called on leaf node");
        }
    }

    fn get_leaf(&self) -> &L {
        if let &NodeVal::Leaf(ref l) = &self.0.val {
            l
        } else {
            panic!("get_leaf called on internal node");
        }
    }

    fn is_ok_child(&self) -> bool {
        match self.0.val {
            NodeVal::Leaf(ref l) => l.is_ok_child(),
            NodeVal::Internal(ref nodes) => (nodes.len() >= MIN_CHILDREN)
        }
    }

    fn merge_nodes(children1: &[Node<N, L>], children2: &[Node<N, L>]) -> Node<N, L> {
        let n_children = children1.len() + children2.len();
        if n_children <= MAX_CHILDREN {
            Node::from_nodes([children1, children2].concat())
        } else {
            // Note: this leans left. Splitting at midpoint is also an option
            let splitpoint = min(MAX_CHILDREN, n_children - MIN_CHILDREN);
            let mut iter = children1.iter().chain(children2.iter()).cloned();
            let left = iter.by_ref().take(splitpoint).collect();
            let right = iter.collect();
            let parent_nodes = vec![Node::from_nodes(left), Node::from_nodes(right)];
            Node::from_nodes(parent_nodes)
        }
    }

    // precondition: both ropes are leaves
    fn merge_leaves(mut rope1: Node<N, L>, rope2: Node<N, L>) -> Node<N, L> {
        let both_ok = rope1.get_leaf().is_ok_child() && rope2.get_leaf().is_ok_child();
        if both_ok {
            return Node::from_nodes(vec![rope1, rope2]);
        }
        match {
            let mut node1 = Arc::make_mut(&mut rope1.0);
            let leaf2 = rope2.get_leaf();
            let len2 = leaf2.len();
            if let NodeVal::Leaf(ref mut leaf1) = node1.val {
                leaf1.push_maybe_split(leaf2, 0, len2 + 1)
            } else {
                panic!("merge_leaves called on non-leaf");
            }
        } {
            Some(new) => {
                Node::from_nodes(vec![
                    rope1,
                    Node::from_leaf(new),
                ])
            }
            None => rope1
        }
    }

    fn concat(rope1: Node<N, L>, rope2: Node<N, L>) -> Node<N, L> {
        let h1 = rope1.height();
        let h2 = rope2.height();
        if h1 == h2 {
            if rope1.is_ok_child() && rope2.is_ok_child() {
                return Node::from_nodes(vec![rope1, rope2]);
            }
            if h1 == 0 {
                return Node::merge_leaves(rope1, rope2);
            }
            return Node::merge_nodes(rope1.get_children(), rope2.get_children());
        } else if h1 < h2 {
            let children2 = rope2.get_children();
            if h1 == h2 - 1 && rope1.is_ok_child() {
                return Node::merge_nodes(&[rope1], children2);
            }
            let newrope = Node::concat(rope1, children2[0].clone());
            if newrope.height() == h2 - 1 {
                return Node::merge_nodes(&[newrope], &children2[1..]);
            } else {
                return Node::merge_nodes(newrope.get_children(), &children2[1..]);
            }
        } else {  // h1 > h2
            let children1 = rope1.get_children();
            if h2 == h1 - 1 && rope2.is_ok_child() {
                return Node::merge_nodes(children1, &[rope2]);
            }
            let lastix = children1.len() - 1;
            let newrope = Node::concat(children1[lastix].clone(), rope2);
            if newrope.height() == h1 - 1 {
                return Node::merge_nodes(&children1[..lastix], &[newrope]);
            } else {
                return Node::merge_nodes(&children1[..lastix], newrope.get_children());
            }
        }
    }

    fn measure<M: Metric<N, L>>(&self) -> usize {
        M::measure(&self.0.info)
    }

    /*
    // calls the given function on with leaves forming the sequence
    fn visit_subseq<M: Metric<N, L>, F>(&self, start: usize, end: usize,
            f: &mut F) where F: FnMut(&L) -> () {
        match self.0.val {
            NodeVal::Leaf(ref l) => {
                if start == 0 && end == self.measure::<M>() {
                    f(&l)
                } else {
                    f(&M::slice(l, start, end))
                }
            }
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    let child_measure = child.measure::<M>();
                    if offset + child_measure > start {
                        child.visit_subseq::<M, F>(max(offset, start) - offset,
                            min(child_measure, end - offset), f);
                    }
                    offset += child_measure;
                }
                return;
            }
        }
    }
    */

    fn push_subseq<M: Metric<N, L>>(&self,
            b: &mut RopeBuilder<N, L>, start: usize, end: usize) {
        if start == 0 && self.measure::<M>() == end {
            b.push(self.clone());
            return
        }
        match self.0.val {
            NodeVal::Leaf(ref l) => {
                let base_start = M::to_base_units(l, start);
                let base_end = M::to_base_units(l, end);
                b.push_leaf_slice(l, base_start, base_end)
            }
            NodeVal::Internal(ref v) => {
                let mut offset = 0;
                for child in v {
                    if end <= offset {
                        break;
                    }
                    let child_measure = child.measure::<M>();
                    if offset + child_measure >= start {
                        child.push_subseq::<M>(b,
                            max(offset, start) - offset,
                            min(child_measure + 1, end - offset));
                    }
                    offset += child_measure;
                }
                return;
            }
        }
    }

    fn convert_metrics<M1: Metric<N, L>, M2: Metric<N, L>>(&self, mut m1: usize) -> usize {
        if m1 == 0 { return 0; }
        // if leaf is guaranteed to have no M1 fragments, could be >=
        // so maybe metric can have bool indicating this?
        if m1 > self.measure::<M1>() {
            return self.measure::<M2>() + 1;
        }
        let mut m2 = 0;
        let mut node = self;
        while node.height() > 0 {
            for child in node.get_children() {
                let child_m1 = child.measure::<M1>();
                // same as above, < if no fragments, could be more efficient
                if m1 <= child_m1 {
                    node = child;
                    break;
                }
                m2 += child.measure::<M2>();
                m1 -= child_m1;
            }
        }
        let l = node.get_leaf();
        let base = M1::to_base_units(l, m1);
        m2 + M2::from_base_units(l, m2)
    }
}

struct RopeBuilder<N: NodeInfo, L: Leaf<N>>(Option<Node<N, L>>);

impl<N: NodeInfo, L: Leaf<N>> RopeBuilder<N, L> {
    fn new() -> RopeBuilder<N, L> {
        RopeBuilder(None)
    }

    fn push(&mut self, n: Node<N, L>) {
        match self.0.take() {
            None => self.0 = Some(n),
            Some(buf) => self.0 = Some(Node::concat(buf, n))
        }
    }

    fn push_leaf(&mut self, l: L) {
        self.push(Node::from_leaf(l))
    }

    fn push_leaf_slice(&mut self, l: &L, start: usize, end: usize) {
        self.push(Node::from_leaf(l.subseq(start, end)))
    }
}

// some concrete instantiations of the traits

#[derive(Clone, Default)]
struct BytesLeaf(Vec<u8>);

#[derive(Clone)]
struct BytesInfo(usize);

impl Leaf<BytesInfo> for BytesLeaf {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn compute_info(&self) -> BytesInfo {
        BytesInfo(self.len())
    }

    fn is_ok_child(&self) -> bool {
        self.0.len() >= 512
    }

    fn push_maybe_split(&mut self, other: &BytesLeaf, start: usize, end: usize) -> Option<BytesLeaf> {
        self.0.extend_from_slice(&other.0[start..end]);
        if self.0.len() <= 1024 {
            None
        } else {
            let splitpoint = self.0.len() / 2;
            let new = self.0[splitpoint..].to_owned();
            self.0.truncate(splitpoint);
            Some(BytesLeaf(new))
        }
    }
}

impl NodeInfo for BytesInfo {
    fn accumulate(&mut self, other: &Self) {
        self.0 += other.0;
    }
}

struct BytesMetric(());

impl Metric<BytesInfo, BytesLeaf> for BytesMetric {
    fn measure(info: &BytesInfo) -> usize {
        info.0
    }

    fn to_base_units(_: &BytesLeaf, in_measured_units: usize) -> usize {
        in_measured_units
    }

    fn from_base_units(_: &BytesLeaf, in_base_units: usize) -> usize {
        in_base_units
    }
}

// Another more interesting example - Points represents a (multi-) set
// of indexes. A motivating use is storing line breaks.

// Here the base units are the underlying indices, ie it should track
// the buffer being broken

#[derive(Clone, Default)]
struct PointsLeaf {
    len: usize,  // measured in base units
    data: Vec<usize>,  // each is a delta relative to start of leaf; sorted
}

#[derive(Clone)]
struct PointsInfo(usize);  // number of breaks

impl Leaf<PointsInfo> for PointsLeaf {
    fn len(&self) -> usize {
        self.len
    }

    fn compute_info(&self) -> PointsInfo {
        PointsInfo(self.data.len())
    }

    fn is_ok_child(&self) -> bool {
        self.data.len() >= 32
    }

    fn push_maybe_split(&mut self, other: &PointsLeaf, start: usize, end: usize) -> Option<PointsLeaf> {
        for &v in other.data.iter() {
            if start <= v && v < end {
                self.data.push(v - start + self.len);
            }
        }
        if self.data.len() <= 64 {
            None
        } else {
            let splitpoint = self.data.len() / 2;  // number of breaks
            let splitpoint_units = self.data[splitpoint - 1];
            let mut new = Vec::with_capacity(self.data.len() - splitpoint);
            for i in splitpoint..self.data.len() {
                new.push(self.data[i] - splitpoint_units);
            }
            let new_len = self.len - splitpoint_units;
            self.len = splitpoint_units;
            self.data.truncate(splitpoint);
            Some(PointsLeaf {
                len: new_len,
                data: new,
            })
        }
    }
}

impl NodeInfo for PointsInfo {
    fn accumulate(&mut self, other: &Self) {
        self.0 += other.0;
    }
}

struct PointsMetric(());

impl Metric<PointsInfo, PointsLeaf> for PointsMetric {
    fn measure(info: &PointsInfo) -> usize {
        info.0
    }

    fn to_base_units(l: &PointsLeaf, in_measured_units: usize) -> usize {
        if in_measured_units > l.data.len() {
            l.len + 1  // I think this is right, but not needed if base is non-frag
        } else if in_measured_units == 0 {
            0
        } else {
            l.data[in_measured_units - 1]
        }
    }

    fn from_base_units(l: &PointsLeaf, in_base_units: usize) -> usize {
        // TODO: binary search, data is sorted
        for i in 0..l.data.len() {
            if in_base_units < l.data[i] {
                return i
            }
        }
        l.data.len()
    }
}

